import { Service, computed, inject, signal } from '@angular/core';

import { RepositoryCatalog, type RepositoryCatalogEntry } from '../repositories/repository-catalog';
import { GitHubAccountStore, type GitHubAccountState } from './github-account.store';
import {
  GITHUB_BRIDGE,
  type GitHubAccount,
  type GitHubPullRequestScope,
  type GitHubPullRequestSummary,
} from './github-bridge';

export interface DashboardPullRequest extends GitHubPullRequestSummary {
  readonly repositoryId: string;
  readonly repositoryName: string;
  readonly repositoryFullName: string;
  readonly reviewRequestedFromViewer: boolean;
}

export interface PullRequestRepositoryFailure {
  readonly repositoryFullName: string;
  readonly message: string;
}

export type PullRequestDashboardState =
  | { readonly kind: 'idle' }
  | { readonly kind: 'loading'; readonly repositoryCount: number }
  | {
      readonly kind: 'ready';
      readonly pullRequests: readonly DashboardPullRequest[];
      readonly repositoryCount: number;
      readonly failures: readonly PullRequestRepositoryFailure[];
    }
  | { readonly kind: 'error'; readonly message: string };

const PAGE_SIZE = 50;
const MAX_PAGES_PER_SCOPE = 5;

@Service()
export class GithubPullRequestDashboard {
  private readonly bridge = inject(GITHUB_BRIDGE);
  private readonly catalog = inject(RepositoryCatalog);
  private readonly accounts = inject(GitHubAccountStore);
  private generation = 0;

  readonly state = signal<PullRequestDashboardState>({ kind: 'idle' });
  readonly showReviewRequested = signal(true);
  readonly showAuthored = signal(true);
  readonly selected = signal<DashboardPullRequest | null>(null);
  readonly account = signal<GitHubAccount | null>(null);
  readonly visiblePullRequests = computed(() => {
    const state = this.state();
    if (state.kind !== 'ready') {
      return [];
    }
    return state.pullRequests.filter((pullRequest) =>
      (this.showReviewRequested() && pullRequest.reviewRequestedFromViewer) ||
      (this.showAuthored() && pullRequest.authoredByViewer),
    );
  });

  setReviewRequestedVisible(visible: boolean): void {
    this.showReviewRequested.set(visible);
  }

  setAuthoredVisible(visible: boolean): void {
    this.showAuthored.set(visible);
  }

  select(pullRequest: DashboardPullRequest | null): void {
    this.selected.set(pullRequest);
  }

  async load(): Promise<void> {
    const generation = ++this.generation;
    this.selected.set(null);
    try {
      await Promise.all([this.catalog.load(), this.ensureAccountsLoaded()]);
      if (generation !== this.generation) {
        return;
      }
      let account = preferredAccount(this.accounts.state());
      if (account === null) {
        await this.accounts.connectCli();
        account = preferredAccount(this.accounts.state());
      }
      if (account === null) {
        this.account.set(null);
        this.state.set({ kind: 'error', message: 'Connect GitHub CLI or another GitHub account to load pull requests.' });
        return;
      }
      this.account.set(account);
      const repositories = mainGitHubRepositories(this.catalog.groups());
      this.state.set({ kind: 'loading', repositoryCount: repositories.length });
      const merged = new Map<string, DashboardPullRequest>();
      const failures: PullRequestRepositoryFailure[] = [];
      for (const repository of repositories) {
        for (const scope of ['assignedToViewer', 'authoredByViewer'] as const) {
          try {
            const pulls = await this.loadRepositoryScope(account.id, repository, scope);
            for (const pull of pulls) {
              const fullName = repositoryFullName(repository);
              const key = `${fullName.toLowerCase()}#${pull.number}`;
              const existing = merged.get(key);
              merged.set(key, {
                ...(existing ?? pull),
                ...pull,
                repositoryId: repository.id,
                repositoryName: repository.name,
                repositoryFullName: fullName,
                authoredByViewer: pull.authoredByViewer || existing?.authoredByViewer === true,
                reviewRequestedFromViewer: scope === 'assignedToViewer' || existing?.reviewRequestedFromViewer === true,
              });
            }
          } catch (error) {
            failures.push({
              repositoryFullName: repositoryFullName(repository),
              message: errorMessage(error),
            });
            break;
          }
        }
      }
      if (generation !== this.generation) {
        return;
      }
      const pullRequests = [...merged.values()].sort((left, right) =>
        Date.parse(right.updatedAt) - Date.parse(left.updatedAt) ||
        left.repositoryFullName.localeCompare(right.repositoryFullName) ||
        right.number - left.number,
      );
      this.state.set({ kind: 'ready', pullRequests, repositoryCount: repositories.length, failures });
    } catch (error) {
      if (generation === this.generation) {
        this.state.set({ kind: 'error', message: errorMessage(error) });
      }
    }
  }

  private async ensureAccountsLoaded(): Promise<void> {
    if (this.accounts.state().kind !== 'ready') {
      await this.accounts.load();
    }
  }

  private async loadRepositoryScope(
    accountId: string,
    repository: RepositoryCatalogEntry,
    scope: GitHubPullRequestScope,
  ): Promise<readonly GitHubPullRequestSummary[]> {
    const pulls: GitHubPullRequestSummary[] = [];
    let cursor: string | null = null;
    for (let page = 0; page < MAX_PAGES_PER_SCOPE; page += 1) {
      const response = await this.bridge.githubListPullRequests({
        accountId,
        repositoryId: repository.id,
        scope,
        cursor,
        pageSize: PAGE_SIZE,
      });
      pulls.push(...response.pullRequests);
      cursor = response.nextCursor;
      if (cursor === null) {
        break;
      }
    }
    return pulls;
  }
}

function preferredAccount(state: GitHubAccountState): GitHubAccount | null {
  if (state.kind !== 'ready') {
    return null;
  }
  return state.accounts.find((account) => account.state === 'connected' && account.authKind === 'gitHubCli') ??
    state.accounts.find((account) => account.state === 'connected') ??
    null;
}

function mainGitHubRepositories(groups: ReturnType<RepositoryCatalog['groups']>): readonly RepositoryCatalogEntry[] {
  const repositories = groups
    .map((group) => group.worktrees.find((repository) => repository.worktreeRole === 'main') ??
      (group.representative.worktreeRole === 'linked' ? null : group.representative))
    .filter((repository): repository is RepositoryCatalogEntry =>
      repository !== null &&
      repository.availability === 'available' &&
      repository.hostedIdentity !== null &&
      repository.hostedIdentity.host.toLowerCase() === 'github.com',
    );
  const seen = new Set<string>();
  return repositories.filter((repository) => {
    const key = repositoryFullName(repository).toLowerCase();
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

function repositoryFullName(repository: RepositoryCatalogEntry): string {
  const identity = repository.hostedIdentity;
  return identity === null ? repository.name : `${identity.owner}/${identity.name}`;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  if (typeof error === 'object' && error !== null && 'message' in error) {
    const message = (error as { readonly message?: unknown }).message;
    if (typeof message === 'string' && message.trim().length > 0) {
      return message;
    }
  }
  return 'Pull requests could not be loaded.';
}
