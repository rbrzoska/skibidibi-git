import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubRepositoryPullRequestStore, type GitHubBridge } from '../../../core/github';
import { GitHubPullRequestInspector } from './github-pull-request-inspector';

describe('GitHubPullRequestInspector', () => {
  it('renders conversation comments and review-thread state', async () => {
    const comment = { id: 'comment-1', authorLogin: 'grace', body: 'Please adjust this.', createdAt: '2026-07-15T12:00:00Z', updatedAt: '2026-07-15T12:00:00Z', url: null, path: null, line: null, side: null };
    const detail = {
      number: 4, title: 'Review me', url: 'https://github.com/o/r/pull/4', state: 'open' as const, draft: false,
      authorLogin: 'ada', headRefName: 'feature', baseRefName: 'main', updatedAt: '2026-07-15T12:00:00Z',
      authoredByViewer: false, reviewRequestedFromViewer: true, unresolvedThreadCount: 1,
      body: 'Description', additions: 4, deletions: 2, changedFiles: 3, mergeability: 'mergeable' as const,
      conversationTruncated: false, reviewThreadsTruncated: false, comments: [comment],
      reviewThreads: [{ id: 'thread-1', path: 'src/app.ts', line: 12, resolved: false, outdated: true, comments: [comment] }],
    };
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(), githubConnectPat: vi.fn(), githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(), githubPullRequestDetail: vi.fn().mockResolvedValue(detail),
    };
    await TestBed.configureTestingModule({
      imports: [GitHubPullRequestInspector],
      providers: [GitHubRepositoryPullRequestStore, { provide: GITHUB_BRIDGE, useValue: bridge }],
    }).compileComponents();
    const store = TestBed.inject(GitHubRepositoryPullRequestStore);
    store.configure('repo-1', 'account-1');
    await store.select(4);
    const fixture = TestBed.createComponent(GitHubPullRequestInspector);
    fixture.detectChanges();

    expect(fixture.nativeElement.textContent).toContain('Review me');
    expect(fixture.nativeElement.textContent).toContain('Please adjust this.');
    expect(fixture.nativeElement.textContent).toContain('src/app.ts:12');
    expect(fixture.nativeElement.textContent).toContain('Outdated');
  });
});
