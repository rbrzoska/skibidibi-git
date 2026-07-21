import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubAccountStore, GitHubRepositoryPullRequestStore, type GitHubBridge } from '../../../core/github';
import { GitHubPullRequestList } from './github-pull-request-list';

describe('GitHubPullRequestList', () => {
  it('renders viewer badges and selects a pull request', async () => {
    const pullRequest = {
      number: 7, title: 'Ship GitHub UI', url: 'https://github.com/o/r/pull/7', state: 'open' as const,
      draft: false, authorLogin: 'ada', headRefName: 'feature', baseRefName: 'main', updatedAt: '2026-07-15T12:00:00Z',
      authoredByViewer: true, reviewRequestedFromViewer: true, unresolvedThreadCount: 2,
      commentCount: 5,
    };
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(), githubStartDeviceFlow: vi.fn(), githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(), githubConnectPat: vi.fn(), githubConnectCli: vi.fn().mockResolvedValue({
        id: 'github-cli:github.com:ada', login: 'ada', host: 'github.com', avatarUrl: null,
        state: 'connected', authKind: 'gitHubCli',
      }), githubDisconnectAccount: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn().mockResolvedValue({ pullRequests: [pullRequest], nextCursor: null }),
      githubPullRequestDetail: vi.fn().mockResolvedValue({ ...pullRequest, body: '', comments: [], reviewThreads: [] }),
    };
    await TestBed.configureTestingModule({
      imports: [GitHubPullRequestList],
      providers: [GitHubAccountStore, GitHubRepositoryPullRequestStore, { provide: GITHUB_BRIDGE, useValue: bridge }],
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
    expect(fixture.nativeElement.querySelector('.comment-count')?.textContent).toContain('5');
    expect(fixture.nativeElement.querySelector('.mine')?.textContent).toContain('Mine');
    expect(fixture.nativeElement.querySelector('.review-requested')?.textContent).toContain('Review requested');
    expect(fixture.nativeElement.querySelector('.unresolved')?.textContent).toContain('2 unresolved');
    expect(fixture.nativeElement.querySelector('.title small')?.textContent).toContain('feature');
    expect(fixture.nativeElement.querySelector('.title small')?.textContent).toContain('main');
    const open = fixture.nativeElement.querySelector('.open-pr') as HTMLAnchorElement;
    expect(open.href).toBe('https://github.com/o/r/pull/7');
    expect(open.target).toBe('_blank');
    expect(open.rel).toBe('noreferrer');
    expect(fixture.nativeElement.querySelector('.github-pull-requests')?.textContent).not.toContain('Pull Requests');
    expect(fixture.nativeElement.querySelector('.pr-count')?.textContent.trim()).toBe('1');
    expect(fixture.nativeElement.querySelector('[aria-pressed="true"]')?.textContent).toContain('Assigned to me');
    expect(bridge.githubListPullRequests).toHaveBeenCalledWith({
      accountId: 'account-1', repositoryId: 'repo-1', scope: 'assignedToViewer', cursor: null, pageSize: 30,
    });
    (fixture.nativeElement.querySelector('li button') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(bridge.githubPullRequestDetail).toHaveBeenCalledWith({ accountId: 'account-1', repositoryId: 'repo-1', number: 7 });

    ([...fixture.nativeElement.querySelectorAll('.pr-scope button')] as HTMLButtonElement[])
      .find((button) => button.textContent?.includes('My PRs'))?.click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelector('[aria-pressed="true"]')?.textContent).toContain('My PRs');
    expect(bridge.githubListPullRequests).toHaveBeenCalledWith({
      accountId: 'account-1', repositoryId: 'repo-1', scope: 'authoredByViewer', cursor: null, pageSize: 30,
    });
    expect(TestBed.inject(GitHubRepositoryPullRequestStore).selectedNumber()).toBeNull();

    (fixture.nativeElement.querySelector('[aria-label="Refresh GitHub CLI authentication and pull requests"]') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(bridge.githubConnectCli).toHaveBeenCalledOnce();
    expect(bridge.githubListPullRequests).toHaveBeenCalledWith({
      accountId: 'github-cli:github.com:ada', repositoryId: 'repo-1', scope: 'authoredByViewer', cursor: null, pageSize: 30,
    });
    expect(fixture.nativeElement.textContent).not.toContain('Check out branch');
  });
});
