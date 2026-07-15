import {
  ChangeDetectionStrategy,
  Component,
  type OnDestroy,
  computed,
  inject,
  signal,
} from '@angular/core';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';

import {
  DESKTOP_IPC,
  type CommitChangedFile,
  type RepositoryCommitDetailResponse,
  type RepositoryFileDiffResponse,
  type RepositoryCommitSummary,
  type RepositoryBranch,
  type RepositoryNavigationResponse,
  type RepositoryWorktree,
} from '../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import { buildBranchTree, visibleBranchTree } from './branch-tree';
import { parseUnifiedDiff } from './unified-diff';

type HistoryPhase = 'idle' | 'loading' | 'ready' | 'error';
type DetailState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly detail: RepositoryCommitDetailResponse }
  | { readonly kind: 'error'; readonly message: string };
type NavigationState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly navigation: RepositoryNavigationResponse }
  | { readonly kind: 'error'; readonly message: string };
type FileDiffState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading'; readonly path: string; readonly oldPath: string | null }
  | { readonly kind: 'ready'; readonly response: RepositoryFileDiffResponse }
  | { readonly kind: 'error'; readonly path: string; readonly oldPath: string | null; readonly message: string };

const HISTORY_PAGE_SIZE = 50;
const MAX_RENDERED_DIFF_ROWS = 20_000;
const SIDEBAR_WIDTH_KEY = 'skibidibi-git.workspace.sidebar-width';
const SIDEBAR_MIN_WIDTH = 220;
const SIDEBAR_DEFAULT_WIDTH = 288;
const SIDEBAR_KEYBOARD_STEP = 16;
const ROOT_FONT_SIZE = 16;
const COMPACT_LAYOUT_BREAKPOINT = 48 * ROOT_FONT_SIZE;
const INSPECTOR_LAYOUT_BREAKPOINT = 68 * ROOT_FONT_SIZE;
const HISTORY_MIN_WIDTH = 24 * ROOT_FONT_SIZE;
const INSPECTOR_MIN_WIDTH = 17 * ROOT_FONT_SIZE;
const SIDEBAR_RESIZER_WIDTH = 0.4 * ROOT_FONT_SIZE;

@Component({
  selector: 'app-workspace-history',
  imports: [RouterLink],
  templateUrl: './workspace-history.html',
  styleUrl: './workspace-history.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class WorkspaceHistory implements OnDestroy {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly catalog = inject(RepositoryCatalog);
  private readonly ipc = inject(DESKTOP_IPC);
  private historyRequestGeneration = 0;
  private detailRequestGeneration = 0;
  private navigationRequestGeneration = 0;
  private fileDiffRequestGeneration = 0;
  private activeResizePointer: number | null = null;

  protected readonly statusStore = inject(RepositoryStatusStore);
  protected readonly repositoryId = this.route.snapshot.paramMap.get('repositoryId') ?? '';
  protected readonly repository = computed(() => this.catalog.find(this.repositoryId));
  protected readonly historyPhase = signal<HistoryPhase>('idle');
  protected readonly historyError = signal('');
  protected readonly commits = signal<readonly RepositoryCommitSummary[]>([]);
  protected readonly nextCursor = signal<string | null>(null);
  protected readonly isLoadingMore = signal(false);
  protected readonly selectedOid = signal<string | null>(null);
  protected readonly detailState = signal<DetailState>({ kind: 'idle' });
  protected readonly navigationState = signal<NavigationState>({ kind: 'loading' });
  protected readonly navigationFilter = signal('');
  protected readonly collapsedBranchFolders = signal<ReadonlySet<string>>(new Set());
  protected readonly sidebarWidth = signal(this.readSidebarWidth());
  protected readonly resizingSidebar = signal(false);
  protected readonly switchingBranch = signal<string | null>(null);
  protected readonly openingWorktree = signal<string | null>(null);
  protected readonly navigationActionError = signal('');
  protected readonly selectedFilePath = signal<string | null>(null);
  protected readonly fileDiffState = signal<FileDiffState>({ kind: 'idle' });
  protected readonly parsedFileDiff = computed(() => {
    const state = this.fileDiffState();
    return state.kind === 'ready' ? parseUnifiedDiff(state.response.patch) : null;
  });
  protected readonly visibleFileDiffRows = computed(
    () => this.parsedFileDiff()?.rows.slice(0, MAX_RENDERED_DIFF_ROWS) ?? [],
  );
  protected readonly isFileDiffVisuallyTruncated = computed(
    () => (this.parsedFileDiff()?.rows.length ?? 0) > MAX_RENDERED_DIFF_ROWS,
  );

  protected readonly localBranches = computed(() => this.branchesOfKind('local'));
  protected readonly remoteBranches = computed(() => this.branchesOfKind('remote'));
  protected readonly currentLocalBranch = computed(
    () => this.localBranches().find((branch) => branch.current) ?? null,
  );
  protected readonly otherLocalBranches = computed(() =>
    this.localBranches().filter((branch) => !branch.current),
  );
  protected readonly orderedWorktrees = computed(() => {
    const state = this.navigationState();
    if (state.kind !== 'ready') {
      return [];
    }
    return [...state.navigation.worktrees].sort(
      (left, right) => Number(this.isCurrentWorktree(right)) - Number(this.isCurrentWorktree(left)),
    );
  });
  protected readonly localBranchTree = computed(() =>
    visibleBranchTree(
      buildBranchTree(this.otherLocalBranches()),
      this.collapsedFoldersFor('local'),
      this.navigationFilter(),
    ),
  );
  protected readonly remoteBranchTree = computed(() =>
    visibleBranchTree(
      buildBranchTree(this.remoteBranches()),
      this.collapsedFoldersFor('remote'),
      this.navigationFilter(),
    ),
  );

  protected readonly branchName = computed(() => {
    const state = this.statusStore.state();
    return state.kind === 'ready' ? (state.status.branch.head ?? 'detached HEAD') : 'loading';
  });

  protected readonly workingTreeSummary = computed(() => {
    const state = this.statusStore.state();
    if (state.kind !== 'ready') {
      return { added: 0, modified: 0, deleted: 0, conflicts: 0, total: 0 };
    }
    const entries = state.status.entries;
    return {
      added: entries.filter(
        ({ indexStatus, worktreeStatus }) =>
          indexStatus === 'added' || worktreeStatus === 'untracked',
      ).length,
      modified: entries.filter(
        ({ indexStatus, worktreeStatus }) =>
          indexStatus === 'modified' || worktreeStatus === 'modified',
      ).length,
      deleted: entries.filter(
        ({ indexStatus, worktreeStatus }) =>
          indexStatus === 'deleted' || worktreeStatus === 'deleted',
      ).length,
      conflicts: entries.filter(({ kind }) => kind === 'unmerged').length,
      total: entries.length,
    };
  });

  protected readonly selectedFileSummary = computed(() => {
    const state = this.detailState();
    if (state.kind !== 'ready') {
      return { files: 0, additions: 0, deletions: 0 };
    }
    return state.detail.files.reduce(
      (summary, file) => ({
        files: summary.files + 1,
        additions: summary.additions + (file.additions ?? 0),
        deletions: summary.deletions + (file.deletions ?? 0),
      }),
      { files: 0, additions: 0, deletions: 0 },
    );
  });

  constructor() {
    globalThis.addEventListener('resize', this.clampSidebarToViewport);
    void this.loadRepository();
  }

  ngOnDestroy(): void {
    this.stopSidebarResize();
    globalThis.removeEventListener('resize', this.clampSidebarToViewport);
  }

  protected async refreshWorkspace(): Promise<void> {
    this.navigationActionError.set('');
    await Promise.all([
      this.statusStore.refresh(),
      this.reloadHistory(),
      this.loadNavigation(),
    ]);
  }

  protected updateNavigationFilter(value: string): void {
    this.navigationFilter.set(value);
  }

  protected toggleBranchFolder(kind: RepositoryBranch['kind'], path: string): void {
    const key = `${kind}:${path}`;
    this.collapsedBranchFolders.update((folders) => {
      const next = new Set(folders);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }

  protected async switchBranch(branch: RepositoryBranch): Promise<void> {
    if (branch.kind !== 'local' || branch.current || this.switchingBranch() !== null) {
      return;
    }
    if (!globalThis.confirm(`Switch the active worktree to “${branch.name}”?`)) {
      return;
    }

    this.navigationActionError.set('');
    this.switchingBranch.set(branch.fullName);
    try {
      await this.ipc.invoke('switch_repository_branch', {
        repositoryId: this.repositoryId,
        fullName: branch.fullName,
      });
      await Promise.all([
        this.statusStore.refresh(),
        this.reloadHistory(),
        this.loadNavigation(),
      ]);
    } catch (error) {
      this.navigationActionError.set(this.errorMessage(error, 'The branch could not be switched.'));
    } finally {
      this.switchingBranch.set(null);
    }
  }

  protected async openWorktree(worktree: RepositoryWorktree): Promise<void> {
    if (this.openingWorktree() !== null || this.worktreeDisabledReason(worktree) !== null) {
      return;
    }
    this.navigationActionError.set('');
    this.openingWorktree.set(worktree.path);
    try {
      const repository = await this.catalog.rememberPath(worktree.path);
      if (repository.id !== this.repositoryId) {
        await this.router.navigateByUrl('/repositories', { skipLocationChange: true });
      }
      await this.router.navigate(['/workspace', repository.id, 'history']);
    } catch (error) {
      this.navigationActionError.set(this.errorMessage(error, 'The worktree could not be opened.'));
    } finally {
      this.openingWorktree.set(null);
    }
  }

  protected startSidebarResize(event: PointerEvent): void {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture can be unavailable in tests and older embedded webviews.
    }
    this.activeResizePointer = event.pointerId;
    this.resizingSidebar.set(true);
    globalThis.addEventListener('pointermove', this.resizeSidebar);
    globalThis.addEventListener('pointerup', this.finishSidebarResize);
    globalThis.addEventListener('pointercancel', this.finishSidebarResize);
    globalThis.addEventListener('blur', this.cancelSidebarResize);
  }

  protected resizeSidebarWithKeyboard(event: KeyboardEvent): void {
    let width: number | null = null;
    if (event.key === 'ArrowLeft') {
      width = this.sidebarWidth() - SIDEBAR_KEYBOARD_STEP;
    } else if (event.key === 'ArrowRight') {
      width = this.sidebarWidth() + SIDEBAR_KEYBOARD_STEP;
    } else if (event.key === 'Home') {
      width = SIDEBAR_MIN_WIDTH;
    } else if (event.key === 'End') {
      width = this.maximumSidebarWidth();
    }
    if (width === null) {
      return;
    }
    event.preventDefault();
    this.setSidebarWidth(width, true);
  }

  protected sidebarMaximumWidth(): number {
    return this.maximumSidebarWidth();
  }

  protected worktreeDisabledReason(worktree: RepositoryWorktree): string | null {
    if (this.isCurrentWorktree(worktree)) {
      return 'current';
    }
    if (worktree.bare) {
      return 'bare';
    }
    if (worktree.prunable) {
      return 'prunable';
    }
    return null;
  }

  protected worktreeBranchLabel(worktree: RepositoryWorktree): string {
    if (worktree.branch !== null) {
      return worktree.branch.replace(/^refs\/heads\//, '');
    }
    return worktree.detached ? 'detached HEAD' : worktree.path;
  }

  protected async loadNavigation(): Promise<void> {
    const generation = ++this.navigationRequestGeneration;
    this.navigationState.set({ kind: 'loading' });
    try {
      const navigation = await this.ipc.invoke('repository_navigation', {
        repositoryId: this.repositoryId,
      });
      if (generation === this.navigationRequestGeneration) {
        this.navigationState.set({ kind: 'ready', navigation });
      }
    } catch {
      if (generation === this.navigationRequestGeneration) {
        this.navigationState.set({
          kind: 'error',
          message: 'Repository references could not be loaded.',
        });
      }
    }
  }

  protected async reloadHistory(): Promise<void> {
    const generation = ++this.historyRequestGeneration;
    ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.commits.set([]);
    this.nextCursor.set(null);
    this.isLoadingMore.set(false);
    this.selectedOid.set(null);
    this.detailState.set({ kind: 'idle' });
    this.selectedFilePath.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.historyError.set('');
    this.historyPhase.set('loading');

    try {
      const response = await this.ipc.invoke('repository_history', {
        repositoryId: this.repositoryId,
        cursor: null,
        limit: HISTORY_PAGE_SIZE,
      });
      if (generation !== this.historyRequestGeneration) {
        return;
      }
      this.commits.set(response.commits);
      this.nextCursor.set(response.nextCursor);
      this.historyPhase.set('ready');
    } catch {
      if (generation === this.historyRequestGeneration) {
        this.historyError.set('Commit history could not be loaded. Try again.');
        this.historyPhase.set('error');
      }
    }
  }

  protected async loadMore(): Promise<void> {
    const cursor = this.nextCursor();
    if (cursor === null || this.isLoadingMore()) {
      return;
    }

    const generation = ++this.historyRequestGeneration;
    this.isLoadingMore.set(true);
    this.historyError.set('');
    try {
      const response = await this.ipc.invoke('repository_history', {
        repositoryId: this.repositoryId,
        cursor,
        limit: HISTORY_PAGE_SIZE,
      });
      if (generation !== this.historyRequestGeneration) {
        return;
      }
      this.commits.update((commits) => [...commits, ...response.commits]);
      this.nextCursor.set(response.nextCursor);
    } catch {
      if (generation === this.historyRequestGeneration) {
        this.historyError.set('More commits could not be loaded. Try again.');
      }
    } finally {
      if (generation === this.historyRequestGeneration) {
        this.isLoadingMore.set(false);
      }
    }
  }

  protected async selectCommit(commit: RepositoryCommitSummary): Promise<void> {
    if (this.selectedOid() === commit.oid && this.detailState().kind === 'ready') {
      return;
    }

    const generation = ++this.detailRequestGeneration;
    ++this.fileDiffRequestGeneration;
    this.selectedOid.set(commit.oid);
    this.selectedFilePath.set(null);
    this.fileDiffState.set({ kind: 'idle' });
    this.detailState.set({ kind: 'loading' });
    try {
      const detail = await this.ipc.invoke('repository_commit_detail', {
        repositoryId: this.repositoryId,
        oid: commit.oid,
      });
      if (generation === this.detailRequestGeneration && this.selectedOid() === commit.oid) {
        this.detailState.set({ kind: 'ready', detail });
      }
    } catch {
      if (generation === this.detailRequestGeneration && this.selectedOid() === commit.oid) {
        this.detailState.set({
          kind: 'error',
          message: 'Commit details could not be loaded. Select the commit to retry.',
        });
      }
    }
  }

  protected async openFileDiff(file: CommitChangedFile): Promise<void> {
    await this.loadFileDiff(file.path, file.oldPath);
  }

  protected async retryFileDiff(path: string, oldPath: string | null): Promise<void> {
    await this.loadFileDiff(path, oldPath);
  }

  private async loadFileDiff(path: string, oldPath: string | null): Promise<void> {
    const oid = this.selectedOid();
    if (oid === null) {
      return;
    }

    const generation = ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(path);
    this.fileDiffState.set({ kind: 'loading', path, oldPath });
    try {
      const response = await this.ipc.invoke('repository_file_diff', {
        repositoryId: this.repositoryId,
        oid,
        path,
        oldPath,
      });
      if (
        generation === this.fileDiffRequestGeneration &&
        this.selectedOid() === oid &&
        this.selectedFilePath() === path
      ) {
        this.fileDiffState.set({ kind: 'ready', response });
      }
    } catch (error) {
      if (
        generation === this.fileDiffRequestGeneration &&
        this.selectedOid() === oid &&
        this.selectedFilePath() === path
      ) {
        this.fileDiffState.set({
          kind: 'error',
          path,
          oldPath,
          message: this.errorMessage(error, 'The file diff could not be loaded.'),
        });
      }
    }
  }

  protected closeFileDiff(): void {
    ++this.fileDiffRequestGeneration;
    this.selectedFilePath.set(null);
    this.fileDiffState.set({ kind: 'idle' });
  }

  protected formatTimestamp(timestamp: string): string {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(timestamp));
  }

  protected shortOid(oid: string): string {
    return oid.slice(0, 7);
  }

  protected fileStatusLabel(file: CommitChangedFile): string {
    return file.binary ? `${file.status}, binary` : file.status;
  }

  protected fileStatusMark(file: CommitChangedFile): string {
    const marks: Record<CommitChangedFile['status'], string> = {
      added: '+',
      modified: '●',
      deleted: '−',
      renamed: '→',
      copied: '⧉',
      typeChanged: 'T',
      unmerged: '!',
      unknown: '?',
    };
    return marks[file.status];
  }

  private branchesOfKind(kind: RepositoryBranch['kind']): readonly RepositoryBranch[] {
    const state = this.navigationState();
    return state.kind === 'ready'
      ? state.navigation.branches.filter((branch) => branch.kind === kind)
      : [];
  }

  private collapsedFoldersFor(kind: RepositoryBranch['kind']): ReadonlySet<string> {
    const prefix = `${kind}:`;
    return new Set(
      [...this.collapsedBranchFolders()]
        .filter((path) => path.startsWith(prefix))
        .map((path) => path.slice(prefix.length)),
    );
  }

  private readonly resizeSidebar = (event: PointerEvent): void => {
    if (event.pointerId === this.activeResizePointer) {
      this.setSidebarWidth(event.clientX, false);
    }
  };

  private readonly finishSidebarResize = (event: PointerEvent): void => {
    if (event.pointerId !== this.activeResizePointer) {
      return;
    }
    this.persistSidebarWidth();
    this.stopSidebarResize();
  };

  private readonly cancelSidebarResize = (): void => {
    if (this.activeResizePointer !== null) {
      this.persistSidebarWidth();
      this.stopSidebarResize();
    }
  };

  private readonly clampSidebarToViewport = (): void => {
    this.setSidebarWidth(this.sidebarWidth(), false);
  };

  private stopSidebarResize(): void {
    this.activeResizePointer = null;
    this.resizingSidebar.set(false);
    globalThis.removeEventListener('pointermove', this.resizeSidebar);
    globalThis.removeEventListener('pointerup', this.finishSidebarResize);
    globalThis.removeEventListener('pointercancel', this.finishSidebarResize);
    globalThis.removeEventListener('blur', this.cancelSidebarResize);
  }

  private setSidebarWidth(width: number, persist: boolean): void {
    this.sidebarWidth.set(Math.min(this.maximumSidebarWidth(), Math.max(SIDEBAR_MIN_WIDTH, width)));
    if (persist) {
      this.persistSidebarWidth();
    }
  }

  private maximumSidebarWidth(): number {
    const viewportWidth = globalThis.innerWidth;
    let reservedWidth = 0;
    if (viewportWidth > INSPECTOR_LAYOUT_BREAKPOINT) {
      reservedWidth = HISTORY_MIN_WIDTH + INSPECTOR_MIN_WIDTH + SIDEBAR_RESIZER_WIDTH;
    } else if (viewportWidth > COMPACT_LAYOUT_BREAKPOINT) {
      reservedWidth = HISTORY_MIN_WIDTH + SIDEBAR_RESIZER_WIDTH;
    }
    const availableWidth = viewportWidth - reservedWidth;
    return Math.max(
      SIDEBAR_MIN_WIDTH,
      Math.floor(Math.min(viewportWidth * 0.5, availableWidth)),
    );
  }

  private isCurrentWorktree(worktree: RepositoryWorktree): boolean {
    return this.repository()?.path === worktree.path;
  }

  private readSidebarWidth(): number {
    try {
      const stored = Number(globalThis.localStorage?.getItem(SIDEBAR_WIDTH_KEY));
      return Number.isFinite(stored) && stored > 0
        ? Math.min(this.maximumSidebarWidth(), Math.max(SIDEBAR_MIN_WIDTH, stored))
        : SIDEBAR_DEFAULT_WIDTH;
    } catch {
      return SIDEBAR_DEFAULT_WIDTH;
    }
  }

  private persistSidebarWidth(): void {
    try {
      globalThis.localStorage?.setItem(SIDEBAR_WIDTH_KEY, String(this.sidebarWidth()));
    } catch {
      // Storage may be unavailable in a hardened webview. Resizing remains usable for the session.
    }
  }

  private errorMessage(error: unknown, fallback: string): string {
    if (typeof error === 'string' && error.trim() !== '') {
      return error;
    }
    if (
      typeof error === 'object' &&
      error !== null &&
      'message' in error &&
      typeof error.message === 'string' &&
      error.message.trim() !== ''
    ) {
      return error.message;
    }
    return fallback;
  }

  private async loadRepository(): Promise<void> {
    await this.catalog.load();
    const repository = this.catalog.find(this.repositoryId);
    if (repository === undefined) {
      this.historyError.set('This remembered repository is no longer available.');
      this.historyPhase.set('error');
      return;
    }

    this.statusStore.setRepositoryPath(repository.path);
    await Promise.all([
      this.statusStore.refresh(),
      this.reloadHistory(),
      this.loadNavigation(),
    ]);
  }
}
