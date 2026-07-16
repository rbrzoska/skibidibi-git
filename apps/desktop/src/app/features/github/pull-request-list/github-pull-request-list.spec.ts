import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubRepositoryPullRequestStore, type GitHubBridge } from '../../../core/github';
import { GitHubPullRequestList } from './github-pull-request-list';

describe('GitHubPullRequestList', () => {
  it('renders viewer badges and selects a pull request', async () => {
    const pullRequest = {
      number: 7, title: 'Ship GitHub UI', url: 'https://github.com/o/r/pull/7', state: 'open' as const,
      draft: false, authorLogin: 'ada', headRefName: 'feature', baseRefName: 'main', updatedAt: '2026-07-15T12:00:00Z',
      authoredByViewer: true, reviewRequestedFromViewer: true, unresolvedThreadCount: 2,
    };
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(), githubStartDeviceFlow: vi.fn(), githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(), githubConnectPat: vi.fn(), githubDisconnectAccount: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn().mockResolvedValue({ pullRequests: [pullRequest], nextCursor: null }),
      githubPullRequestDetail: vi.fn().mockResolvedValue({ ...pullRequest, body: '', comments: [], reviewThreads: [] }),
    };
    await TestBed.configureTestingModule({
      imports: [GitHubPullRequestList],
      providers: [GitHubRepositoryPullRequestStore, { provide: GITHUB_BRIDGE, useValue: bridge }],
    }).compileComponents();
    const fixture = TestBed.createComponent(GitHubPullRequestList);
    fixture.componentRef.setInput('repositoryId', 'repo-1');
    fixture.componentRef.setInput('accountId', 'account-1');
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.textContent).toContain('Mine');
    expect(fixture.nativeElement.textContent).toContain('Review requested');
    expect(fixture.nativeElement.textContent).toContain('2 unresolved');
    (fixture.nativeElement.querySelector('li button') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(bridge.githubPullRequestDetail).toHaveBeenCalledWith({ accountId: 'account-1', repositoryId: 'repo-1', number: 7 });
  });
});
