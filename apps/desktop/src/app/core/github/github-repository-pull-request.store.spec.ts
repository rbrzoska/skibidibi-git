import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, type GitHubBridge, type GitHubPullRequestDetail, type GitHubPullRequestSummary } from './github-bridge';
import { GitHubRepositoryPullRequestStore } from './github-repository-pull-request.store';

function summary(number: number): GitHubPullRequestSummary {
  return {
    number,
    title: `Pull request ${number}`,
    url: `https://github.com/o/r/pull/${number}`,
    state: 'open',
    draft: false,
    authorLogin: 'ada',
    headRefName: `feature-${number}`,
    baseRefName: 'main',
    updatedAt: '2026-07-15T12:00:00Z',
    authoredByViewer: number === 1,
    reviewRequestedFromViewer: false,
    unresolvedThreadCount: 0,
  };
}

function detail(number: number): GitHubPullRequestDetail {
  return {
    ...summary(number), body: 'Body', additions: 1, deletions: 2, changedFiles: 3,
    mergeability: 'mergeable', comments: [], reviewThreads: [],
    conversationTruncated: false, reviewThreadsTruncated: false,
  };
}

describe('GitHubRepositoryPullRequestStore', () => {
  it('loads deduplicated cursor pages for the configured repository and account', async () => {
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn()
        .mockResolvedValueOnce({ pullRequests: [summary(1)], nextCursor: 'page-2' })
        .mockResolvedValueOnce({ pullRequests: [summary(1), summary(2)], nextCursor: null }),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubRepositoryPullRequestStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubRepositoryPullRequestStore);
    store.configure('repo-1', 'account-1');

    await store.load();
    await store.loadMore();

    const state = store.listState();
    expect(state).toMatchObject({ kind: 'ready', nextCursor: null });
    expect(state.kind === 'ready' ? state.pullRequests.map(({ number }) => number) : []).toEqual([1, 2]);
    expect(bridge.githubListPullRequests).toHaveBeenNthCalledWith(2, {
      repositoryId: 'repo-1', accountId: 'account-1', cursor: 'page-2', pageSize: 30,
    });
  });

  it('ignores stale list and detail responses after context or selection changes', async () => {
    let resolveOldList!: (value: { pullRequests: readonly GitHubPullRequestSummary[]; nextCursor: null }) => void;
    let resolveOldDetail!: (value: GitHubPullRequestDetail) => void;
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn()
        .mockReturnValueOnce(new Promise((resolve) => { resolveOldList = resolve; }))
        .mockResolvedValueOnce({ pullRequests: [summary(2)], nextCursor: null }),
      githubPullRequestDetail: vi.fn()
        .mockReturnValueOnce(new Promise((resolve) => { resolveOldDetail = resolve; }))
        .mockResolvedValueOnce(detail(2)),
    };
    TestBed.configureTestingModule({ providers: [GitHubRepositoryPullRequestStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubRepositoryPullRequestStore);
    store.configure('old-repo', 'account-1');
    const oldList = store.load();
    const oldDetail = store.select(1);
    store.configure('new-repo', 'account-1');
    await store.load();
    await store.select(2);

    resolveOldList({ pullRequests: [summary(1)], nextCursor: null });
    resolveOldDetail(detail(1));
    await Promise.all([oldList, oldDetail]);

    const state = store.listState();
    expect(state.kind === 'ready' ? state.pullRequests[0].number : null).toBe(2);
    expect(store.detailState()).toEqual({ kind: 'ready', detail: detail(2) });
  });
});
