import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { ActivatedRoute, RouterLink } from '@angular/router';

import {
  DESKTOP_IPC,
  type CommitChangedFile,
  type RepositoryBranch,
  type RepositoryCompareRefFileDiffResponse,
  type RepositoryCompareRefsRequest,
  type RepositoryCompareRefsResponse,
} from '../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { createContextualDiffRows, type ContextualDiffRow } from '../workspace-history/contextual-diff';
import { splitFilePath } from '../workspace-history/file-path-parts';
import { readReleaseBranch, type ReleaseBranchStorage } from '../workspace-history/release-branch-state';
import { parseUnifiedDiff } from '../workspace-history/unified-diff';

type ComparisonState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly comparison: RepositoryCompareRefsResponse }
  | { readonly kind: 'error'; readonly message: string };

type FileDiffState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading'; readonly path: string }
  | { readonly kind: 'ready'; readonly response: RepositoryCompareRefFileDiffResponse }
  | { readonly kind: 'error'; readonly path: string; readonly message: string };

type ComparisonTab = 'commits' | 'files';

function isSelectableRef(branch: RepositoryBranch): boolean {
  return branch.symbolicTarget === null;
}

function messageFrom(error: unknown, fallback: string): string {
  return typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string'
    ? error.message
    : fallback;
}

@Component({
  selector: 'app-ref-comparison',
  imports: [RouterLink],
  templateUrl: './ref-comparison.html',
  styleUrl: './ref-comparison.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class RefComparison {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly route = inject(ActivatedRoute);
  protected readonly catalog = inject(RepositoryCatalog);
  private readonly releaseBranchStorage: ReleaseBranchStorage = globalThis.localStorage;

  protected readonly repositoryId = this.route.snapshot.paramMap.get('repositoryId') ?? '';
  protected readonly repository = computed(() => this.catalog.find(this.repositoryId));
  protected readonly loadingRefs = signal(true);
  protected readonly references = signal<readonly RepositoryBranch[]>([]);
  protected readonly sourceFullName = signal('');
  protected readonly targetFullName = signal('');
  protected readonly tab = signal<ComparisonTab>('commits');
  protected readonly comparisonState = signal<ComparisonState>({ kind: 'idle' });
  protected readonly fileDiffState = signal<FileDiffState>({ kind: 'idle' });
  protected readonly diffDisplay = signal<'contextual' | 'full'>('contextual');

  protected readonly source = computed(() => this.findReference(this.sourceFullName()));
  protected readonly target = computed(() => this.findReference(this.targetFullName()));
  protected readonly canCompare = computed(() =>
    !this.loadingRefs() &&
    this.source() !== null &&
    this.target() !== null &&
    this.sourceFullName() !== this.targetFullName(),
  );
  protected readonly comparison = computed(() => {
    const state = this.comparisonState();
    return state.kind === 'ready' ? state.comparison : null;
  });
  protected readonly selectedDiffPath = computed(() => {
    const state = this.fileDiffState();
    return state.kind === 'idle'
      ? null
      : state.kind === 'ready'
        ? state.response.path
        : state.path;
  });
  protected readonly parsedDiff = computed(() => {
    const state = this.fileDiffState();
    return state.kind === 'ready' ? parseUnifiedDiff(state.response.patch) : null;
  });
  protected readonly displayedDiffRows = computed<readonly ContextualDiffRow[]>(() => {
    const parsed = this.parsedDiff();
    if (parsed === null) return [];
    return this.diffDisplay() === 'full'
      ? parsed.rows
      : createContextualDiffRows(parsed.rows);
  });

  private comparisonGeneration = 0;
  private fileDiffGeneration = 0;

  constructor() {
    void this.initialize();
  }

  protected selectSource(fullName: string): void {
    this.sourceFullName.set(fullName);
    this.ensureDistinctSelections('source');
    this.invalidateComparison();
  }

  protected selectTarget(fullName: string): void {
    this.targetFullName.set(fullName);
    this.ensureDistinctSelections('target');
    this.invalidateComparison();
  }

  protected swapReferences(): void {
    const source = this.sourceFullName();
    this.sourceFullName.set(this.targetFullName());
    this.targetFullName.set(source);
    this.invalidateComparison();
  }

  protected selectTab(tab: ComparisonTab): void {
    this.tab.set(tab);
  }

  protected async compare(): Promise<void> {
    const request = this.request();
    if (request === null || this.loadingRefs()) return;
    const generation = ++this.comparisonGeneration;
    ++this.fileDiffGeneration;
    this.fileDiffState.set({ kind: 'idle' });
    this.comparisonState.set({ kind: 'loading' });
    try {
      const comparison = await this.ipc.invoke('repository_compare_refs', request);
      if (generation !== this.comparisonGeneration || !comparisonMatchesRequest(comparison, this.request())) return;
      this.comparisonState.set({ kind: 'ready', comparison });
    } catch (error) {
      if (generation === this.comparisonGeneration) {
        this.comparisonState.set({
          kind: 'error',
          message: messageFrom(error, 'The selected references could not be compared.'),
        });
      }
    }
  }

  protected async openFileDiff(file: CommitChangedFile): Promise<void> {
    const request = this.request();
    if (request === null || this.comparison() === null) return;
    const generation = ++this.fileDiffGeneration;
    this.fileDiffState.set({ kind: 'loading', path: file.path });
    this.diffDisplay.set('contextual');
    try {
      const response = await this.ipc.invoke('repository_compare_ref_file_diff', {
        ...request,
        path: file.path,
        oldPath: file.oldPath,
      });
      if (generation !== this.fileDiffGeneration || !fileDiffMatchesRequest(response, this.request(), file)) return;
      this.fileDiffState.set({ kind: 'ready', response });
    } catch (error) {
      if (generation === this.fileDiffGeneration) {
        this.fileDiffState.set({
          kind: 'error',
          path: file.path,
          message: messageFrom(error, 'The file diff could not be loaded.'),
        });
      }
    }
  }

  protected closeFileDiff(): void {
    ++this.fileDiffGeneration;
    this.fileDiffState.set({ kind: 'idle' });
  }

  protected toggleDiffDisplay(): void {
    this.diffDisplay.update((value) => value === 'contextual' ? 'full' : 'contextual');
  }

  protected retryFileDiff(): void {
    const state = this.fileDiffState();
    const comparison = this.comparison();
    if (state.kind !== 'error' || comparison === null) return;
    const file = comparison.files.find((candidate) => candidate.path === state.path);
    if (file !== undefined) void this.openFileDiff(file);
  }

  protected filePathParts = splitFilePath;

  protected shortOid(oid: string): string {
    return oid.slice(0, 8);
  }

  protected formatDate(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.getTime())
      ? value
      : new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(date);
  }

  private async initialize(): Promise<void> {
    try {
      await this.catalog.load();
      if (this.repository() === undefined) {
        this.comparisonState.set({ kind: 'error', message: 'This repository is no longer available.' });
        return;
      }
      const navigation = await this.ipc.invoke('repository_navigation', { repositoryId: this.repositoryId });
      const references = navigation.branches.filter(isSelectableRef);
      this.references.set(references);
      this.applyInitialReferences(references);
    } catch (error) {
      this.comparisonState.set({
        kind: 'error',
        message: messageFrom(error, 'Repository references could not be loaded.'),
      });
    } finally {
      this.loadingRefs.set(false);
    }
  }

  private applyInitialReferences(references: readonly RepositoryBranch[]): void {
    const localRefs = references.filter(({ kind }) => kind === 'local');
    const querySource = this.route.snapshot.queryParamMap.get('source');
    const queryTarget = this.route.snapshot.queryParamMap.get('target');
    const current = localRefs.find(({ current }) => current) ?? null;
    const source = this.refByFullName(references, querySource)
      ?? current
      ?? references[0]
      ?? null;
    const availableLocal = new Set(localRefs.map(({ fullName }) => fullName));
    const release = readReleaseBranch(this.releaseBranchStorage, this.repositoryId, availableLocal);
    const target = [queryTarget, release, 'refs/heads/main', 'refs/heads/master']
      .map((fullName) => this.refByFullName(references, fullName))
      .find((branch) => branch !== null && branch.fullName !== source?.fullName)
      ?? references.find((branch) => branch.fullName !== source?.fullName)
      ?? null;
    this.sourceFullName.set(source?.fullName ?? '');
    this.targetFullName.set(target?.fullName ?? '');
  }

  private ensureDistinctSelections(changed: 'source' | 'target'): void {
    if (this.sourceFullName() !== this.targetFullName()) return;
    const alternate = this.references().find((branch) => branch.fullName !== this.sourceFullName());
    if (alternate === undefined) return;
    if (changed === 'source') {
      this.targetFullName.set(alternate.fullName);
    } else {
      this.sourceFullName.set(alternate.fullName);
    }
  }

  private findReference(fullName: string): RepositoryBranch | null {
    return this.references().find((branch) => branch.fullName === fullName) ?? null;
  }

  private refByFullName(
    references: readonly RepositoryBranch[],
    fullName: string | null,
  ): RepositoryBranch | null {
    return fullName === null ? null : references.find((branch) => branch.fullName === fullName) ?? null;
  }

  private request(): RepositoryCompareRefsRequest | null {
    const source = this.source();
    const target = this.target();
    if (source === null || target === null || source.fullName === target.fullName) return null;
    return {
      repositoryId: this.repositoryId,
      sourceFullName: source.fullName,
      expectedSourceOid: source.oid,
      targetFullName: target.fullName,
      expectedTargetOid: target.oid,
    };
  }

  private invalidateComparison(): void {
    ++this.comparisonGeneration;
    ++this.fileDiffGeneration;
    this.comparisonState.set({ kind: 'idle' });
    this.fileDiffState.set({ kind: 'idle' });
  }
}

interface ComparisonIdentity {
  readonly sourceFullName: string;
  readonly sourceOid: string;
  readonly targetFullName: string;
  readonly targetOid: string;
}

function comparisonMatchesRequest(
  comparison: ComparisonIdentity,
  request: RepositoryCompareRefsRequest | null,
): boolean {
  return request !== null &&
    comparison.sourceFullName === request.sourceFullName &&
    comparison.sourceOid === request.expectedSourceOid &&
    comparison.targetFullName === request.targetFullName &&
    comparison.targetOid === request.expectedTargetOid;
}

function fileDiffMatchesRequest(
  response: RepositoryCompareRefFileDiffResponse,
  request: RepositoryCompareRefsRequest | null,
  file: CommitChangedFile,
): boolean {
  return comparisonMatchesRequest(response, request) &&
    response.path === file.path &&
    response.oldPath === file.oldPath;
}
