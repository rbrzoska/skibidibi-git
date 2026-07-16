import { Injectable, computed, inject, signal } from '@angular/core';

import {
  GITHUB_BRIDGE,
  type GitHubPullRequestDetail,
  type GitHubPullRequestSummary,
} from './github-bridge';

export type GitHubPullRequestListState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading' }
  | {
      readonly kind: 'ready';
      readonly pullRequests: readonly GitHubPullRequestSummary[];
      readonly nextCursor: string | null;
    }
  | { readonly kind: 'error'; readonly message: string };

export type GitHubPullRequestDetailState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading'; readonly number: number }
  | { readonly kind: 'ready'; readonly detail: GitHubPullRequestDetail }
  | { readonly kind: 'error'; readonly number: number; readonly message: string };

const PAGE_SIZE = 30;

@Injectable()
export class GitHubRepositoryPullRequestStore {
  private readonly bridge = inject(GITHUB_BRIDGE);
  private listGeneration = 0;
  private detailGeneration = 0;
  private repositoryId: string | null = null;
  private accountId: string | null = null;

  readonly listState = signal<GitHubPullRequestListState>({ kind: 'idle' });
  readonly detailState = signal<GitHubPullRequestDetailState>({ kind: 'idle' });
  readonly loadingMore = signal(false);
  readonly paginationError = signal('');
  readonly selectedNumber = computed(() => {
    const detail = this.detailState();
    return detail.kind === 'loading' || detail.kind === 'ready' || detail.kind === 'error'
      ? (detail.kind === 'ready' ? detail.detail.number : detail.number)
      : null;
  });

  configure(repositoryId: string, accountId: string | null): boolean {
    const normalizedRepositoryId = repositoryId.trim();
    const normalizedAccountId = accountId?.trim() || null;
    if (this.repositoryId === normalizedRepositoryId && this.accountId === normalizedAccountId) {
      return false;
    }
    this.repositoryId = normalizedRepositoryId;
    this.accountId = normalizedAccountId;
    ++this.listGeneration;
    ++this.detailGeneration;
    this.loadingMore.set(false);
    this.paginationError.set('');
    this.listState.set({ kind: 'idle' });
    this.detailState.set({ kind: 'idle' });
    return true;
  }

  async load(): Promise<void> {
    const context = this.context();
    if (context === null) {
      this.listState.set({ kind: 'idle' });
      return;
    }
    const generation = ++this.listGeneration;
    this.listState.set({ kind: 'loading' });
    this.paginationError.set('');
    try {
      const response = await this.bridge.githubListPullRequests({
        ...context,
        cursor: null,
        pageSize: PAGE_SIZE,
      });
      if (generation === this.listGeneration && this.matches(context)) {
        this.listState.set({ kind: 'ready', pullRequests: response.pullRequests, nextCursor: response.nextCursor });
      }
    } catch (error) {
      if (generation === this.listGeneration && this.matches(context)) {
        this.listState.set({ kind: 'error', message: errorMessage(error, 'Pull requests could not be loaded.') });
      }
    }
  }

  async loadMore(): Promise<void> {
    const context = this.context();
    const current = this.listState();
    if (context === null || current.kind !== 'ready' || current.nextCursor === null || this.loadingMore()) {
      return;
    }
    const generation = this.listGeneration;
    const cursor = current.nextCursor;
    this.loadingMore.set(true);
    this.paginationError.set('');
    try {
      const response = await this.bridge.githubListPullRequests({
        ...context,
        cursor,
        pageSize: PAGE_SIZE,
      });
      if (generation === this.listGeneration && this.matches(context)) {
        const latest = this.listState();
        if (latest.kind === 'ready' && latest.nextCursor === cursor) {
          const existing = new Set(latest.pullRequests.map(({ number }) => number));
          this.listState.set({
            kind: 'ready',
            pullRequests: [...latest.pullRequests, ...response.pullRequests.filter(({ number }) => !existing.has(number))],
            nextCursor: response.nextCursor,
          });
        }
      }
    } catch (error) {
      if (generation === this.listGeneration && this.matches(context)) {
        this.paginationError.set(errorMessage(error, 'More pull requests could not be loaded.'));
      }
    } finally {
      if (generation === this.listGeneration) {
        this.loadingMore.set(false);
      }
    }
  }

  async select(number: number): Promise<void> {
    const context = this.context();
    if (context === null || number <= 0) {
      return;
    }
    const generation = ++this.detailGeneration;
    this.detailState.set({ kind: 'loading', number });
    try {
      const detail = await this.bridge.githubPullRequestDetail({ ...context, number });
      if (generation === this.detailGeneration && this.matches(context)) {
        this.detailState.set({ kind: 'ready', detail });
      }
    } catch (error) {
      if (generation === this.detailGeneration && this.matches(context)) {
        this.detailState.set({ kind: 'error', number, message: errorMessage(error, 'Pull request details could not be loaded.') });
      }
    }
  }

  clearSelection(): void {
    ++this.detailGeneration;
    this.detailState.set({ kind: 'idle' });
  }

  private context(): { readonly repositoryId: string; readonly accountId: string } | null {
    return this.repositoryId !== null && this.repositoryId.length > 0 && this.accountId !== null
      ? { repositoryId: this.repositoryId, accountId: this.accountId }
      : null;
  }

  private matches(context: { readonly repositoryId: string; readonly accountId: string }): boolean {
    return this.repositoryId === context.repositoryId && this.accountId === context.accountId;
  }
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  if (typeof error === 'object' && error !== null && 'message' in error) {
    const message = (error as { readonly message?: unknown }).message;
    if (typeof message === 'string' && message.trim().length > 0) {
      return message;
    }
  }
  return fallback;
}
