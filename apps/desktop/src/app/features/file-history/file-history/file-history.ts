import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { ActivatedRoute, RouterLink } from '@angular/router';

import {
  DESKTOP_IPC,
  type RepositoryFileBlameResponse,
  type RepositoryFileHistoryResponse,
} from '../../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../../core/repositories/repository-catalog';
import { splitFilePath } from '../../workspace-history/file-path-parts';

type LoadState<T> =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly value: T }
  | { readonly kind: 'error'; readonly message: string };

type Tab = 'history' | 'blame';

function errorMessage(error: unknown, fallback: string): string {
  return typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string'
    ? error.message
    : fallback;
}

function isCommitOid(value: string): boolean {
  return /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(value);
}

@Component({
  selector: 'app-file-history',
  imports: [RouterLink],
  templateUrl: './file-history.html',
  styleUrl: './file-history.css',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class FileHistory {
  private readonly ipc = inject(DESKTOP_IPC);
  private readonly route = inject(ActivatedRoute);
  protected readonly catalog = inject(RepositoryCatalog);

  protected readonly repositoryId = this.route.snapshot.paramMap.get('repositoryId') ?? '';
  protected readonly oid = this.route.snapshot.queryParamMap.get('oid') ?? '';
  protected readonly path = this.route.snapshot.queryParamMap.get('path') ?? '';
  protected readonly repository = computed(() => this.catalog.find(this.repositoryId));
  protected readonly pathParts = computed(() => splitFilePath(this.path));
  protected readonly tab = signal<Tab>('history');
  protected readonly historyState = signal<LoadState<RepositoryFileHistoryResponse>>({ kind: 'idle' });
  protected readonly blameState = signal<LoadState<RepositoryFileBlameResponse>>({ kind: 'idle' });
  protected readonly loadingMore = signal(false);
  protected readonly paginationError = signal<string | null>(null);

  private historyGeneration = 0;
  private blameGeneration = 0;

  constructor() {
    void this.initialize();
  }

  protected selectTab(tab: Tab): void {
    this.tab.set(tab);
    if (tab === 'blame' && this.blameState().kind === 'idle') void this.loadBlame();
  }

  protected retryHistory(): void {
    void this.loadHistory();
  }

  protected retryBlame(): void {
    void this.loadBlame();
  }

  protected retryMore(): void {
    void this.loadMore();
  }

  protected async loadMore(): Promise<void> {
    const state = this.historyState();
    if (state.kind !== 'ready' || state.value.nextCursor === null || this.loadingMore()) return;
    const generation = this.historyGeneration;
    this.loadingMore.set(true);
    this.paginationError.set(null);
    try {
      const page = await this.ipc.invoke('repository_file_history', {
        repositoryId: this.repositoryId,
        startOid: this.oid,
        path: this.path,
        cursor: state.value.nextCursor,
      });
      if (generation !== this.historyGeneration) return;
      if (!historyMatchesSnapshot(page, this.oid, this.path)) {
        this.paginationError.set('The file snapshot changed while loading this page. Reload the history to continue.');
        return;
      }
      this.historyState.set({
        kind: 'ready',
        value: { ...page, commits: [...state.value.commits, ...page.commits] },
      });
    } catch (error) {
      if (generation === this.historyGeneration) {
        this.paginationError.set(errorMessage(error, 'The next history page could not be loaded.'));
      }
    } finally {
      if (generation === this.historyGeneration) this.loadingMore.set(false);
    }
  }

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
    } catch {
      // Native file reads remain authoritative; a missing catalog label is non-fatal.
    }
    if (!isCommitOid(this.oid) || this.path.length === 0 || this.path.includes('\0')) {
      this.historyState.set({ kind: 'error', message: 'Open file history from a file in the workspace or commit inspector.' });
      return;
    }
    await this.loadHistory();
  }

  private async loadHistory(): Promise<void> {
    const generation = ++this.historyGeneration;
    this.loadingMore.set(false);
    this.paginationError.set(null);
    this.historyState.set({ kind: 'loading' });
    try {
      const value = await this.ipc.invoke('repository_file_history', {
        repositoryId: this.repositoryId,
        startOid: this.oid,
        path: this.path,
        cursor: null,
      });
      if (generation === this.historyGeneration) {
        this.historyState.set(historyMatchesSnapshot(value, this.oid, this.path)
          ? { kind: 'ready', value }
          : { kind: 'error', message: 'The file snapshot changed while loading history. Try again.' });
      }
    } catch (error) {
      if (generation === this.historyGeneration) {
        this.historyState.set({ kind: 'error', message: errorMessage(error, 'The file history could not be loaded.') });
      }
    }
  }

  private async loadBlame(): Promise<void> {
    const generation = ++this.blameGeneration;
    this.blameState.set({ kind: 'loading' });
    try {
      const value = await this.ipc.invoke('repository_file_blame', {
        repositoryId: this.repositoryId,
        oid: this.oid,
        path: this.path,
      });
      if (generation === this.blameGeneration) {
        this.blameState.set(blameMatchesSnapshot(value, this.oid, this.path)
          ? { kind: 'ready', value }
          : { kind: 'error', message: 'The file snapshot changed while loading blame. Try again.' });
      }
    } catch (error) {
      if (generation === this.blameGeneration) {
        this.blameState.set({ kind: 'error', message: errorMessage(error, 'Blame could not be loaded.') });
      }
    }
  }
}

function historyMatchesSnapshot(
  response: RepositoryFileHistoryResponse,
  oid: string,
  path: string,
): boolean {
  return response.startOid === oid && response.path === path;
}

function blameMatchesSnapshot(
  response: RepositoryFileBlameResponse,
  oid: string,
  path: string,
): boolean {
  return response.oid === oid && response.path === path;
}
