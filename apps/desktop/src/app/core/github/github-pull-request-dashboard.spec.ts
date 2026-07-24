import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { vi } from 'vitest';

import { RepositoryCatalog, type RepositoryCatalogEntry, type RepositoryCatalogGroup } from '../repositories/repository-catalog';
import { GitHubAccountStore } from './github-account.store';
import { GITHUB_BRIDGE, type GitHubBridge, type GitHubPullRequestSummary } from './github-bridge';
import { GithubPullRequestDashboard } from './github-pull-request-dashboard';

const account = {
  id: 'cli-1',
  login: 'octocat',
  host: 'github.com',
  avatarUrl: null,
  state: 'connected' as const,
  authKind: 'gitHubCli' as const,
};

function repository(id: string, role: RepositoryCatalogEntry['worktreeRole']): RepositoryCatalogEntry {
  return {
    id,
    repositoryGroupId: 'group-1',
    worktreeRole: role,
    name: id,
    path: `/work/${id}`,
    provider: 'github',
    transport: 'ssh',
    hostedIdentity: { host: 'github.com', owner: 'acme', name: 'widget' },
    remote: 'github.com/acme/widget',
    integration: 'connected',
    availability: 'available',
    pinned: false,
    lastOpenedAt: null,
    lastOpenedLabel: 'Never',
  };
}

function pull(number: number, overrides: Partial<GitHubPullRequestSummary> = {}): GitHubPullRequestSummary {
  return {
    number,
    title: `Pull request ${number}`,
    url: `https://github.com/acme/widget/pull/${number}`,
    state: 'open',
    draft: false,
    authorLogin: number === 2 ? 'octocat' : 'contributor',
    headRefName: `feature/${number}`,
    baseRefName: 'main',
    updatedAt: `2026-07-${20 + number}T10:00:00Z`,
    authoredByViewer: number === 2,
    commentCount: number,
    approvalCount: number - 1,
    reviewRequestedFromViewer: null,
    unresolvedThreadCount: null,
    ...overrides,
  };
}

describe('GithubPullRequestDashboard', () => {
  it('loads only the main repository and merges review-requested with authored results', async () => {
    const main = repository('main-repository', 'main');
    const linked = repository('linked-worktree', 'linked');
    const group: RepositoryCatalogGroup = {
      id: 'group-1',
      repositoryGroupId: 'group-1',
      grouped: true,
      displayName: 'widget',
      representative: main,
      defaultRepository: main,
      worktrees: [main, linked],
      submodules: [],
      submodulePath: null,
      pinned: false,
      lastOpenedAt: null,
    };
    const bridge = {
      githubListPullRequests: vi.fn().mockImplementation(async ({ scope }: { scope: string }) => ({
        pullRequests: scope === 'assignedToViewer' ? [pull(1)] : [pull(1), pull(2)],
        nextCursor: null,
      })),
    } as unknown as GitHubBridge;
    const accountState = signal({ kind: 'ready' as const, accounts: [account] });
    const accountStore = { state: accountState, load: vi.fn() } as unknown as GitHubAccountStore;
    const catalog = { load: vi.fn(), groups: signal([group]) } as unknown as RepositoryCatalog;
    TestBed.configureTestingModule({
      providers: [
        GithubPullRequestDashboard,
        { provide: GITHUB_BRIDGE, useValue: bridge },
        { provide: GitHubAccountStore, useValue: accountStore },
        { provide: RepositoryCatalog, useValue: catalog },
      ],
    });

    const dashboard = TestBed.inject(GithubPullRequestDashboard);
    await dashboard.load();

    expect(bridge.githubListPullRequests).toHaveBeenCalledTimes(2);
    expect(bridge.githubListPullRequests).toHaveBeenCalledWith(expect.objectContaining({
      repositoryId: main.id,
      pageSize: 50,
    }));
    expect(bridge.githubListPullRequests).not.toHaveBeenCalledWith(expect.objectContaining({ repositoryId: linked.id }));
    expect(dashboard.visiblePullRequests().map(({ number }) => number)).toEqual([2, 1]);
    expect(dashboard.visiblePullRequests().find(({ number }) => number === 1)).toEqual(expect.objectContaining({
      authoredByViewer: false,
      reviewRequestedFromViewer: true,
      approvalCount: 0,
    }));
    expect(dashboard.state()).toEqual(expect.objectContaining({ kind: 'ready', repositoryCount: 1, failures: [] }));
  });

  it('filters the combined inbox without refetching', async () => {
    const main = repository('main-repository', 'main');
    const group = {
      id: 'group-1', repositoryGroupId: 'group-1', grouped: false, displayName: 'widget',
      representative: main, defaultRepository: main, worktrees: [main], submodules: [],
      submodulePath: null, pinned: false, lastOpenedAt: null,
    } as RepositoryCatalogGroup;
    const bridge = {
      githubListPullRequests: vi.fn().mockImplementation(async ({ scope }: { scope: string }) => ({
        pullRequests: scope === 'assignedToViewer' ? [pull(1)] : [pull(2)], nextCursor: null,
      })),
    } as unknown as GitHubBridge;
    TestBed.configureTestingModule({ providers: [
      GithubPullRequestDashboard,
      { provide: GITHUB_BRIDGE, useValue: bridge },
      { provide: GitHubAccountStore, useValue: { state: signal({ kind: 'ready', accounts: [account] }), load: vi.fn() } },
      { provide: RepositoryCatalog, useValue: { load: vi.fn(), groups: signal([group]) } },
    ] });
    const dashboard = TestBed.inject(GithubPullRequestDashboard);
    await dashboard.load();

    dashboard.setReviewRequestedVisible(false);

    expect(dashboard.visiblePullRequests().map(({ number }) => number)).toEqual([2]);
    expect(bridge.githubListPullRequests).toHaveBeenCalledTimes(2);
  });
});
