import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubRepositoryPullRequestStore, type GitHubBridge } from '../../../core/github';
import { GitHubPullRequestInspector } from './github-pull-request-inspector';

describe('GitHubPullRequestInspector', () => {
  it('renders conversation comments and review-thread state', async () => {
    const comment = {
      id: 'comment-1', authorLogin: 'grace', body: '**Please** adjust this.',
      createdAt: '2026-07-15T12:00:00Z', updatedAt: '2026-07-15T12:00:00Z',
      url: null, path: 'src/app.ts', line: 12, side: 'right' as const,
      diffHunk: '@@ -10,3 +10,4 @@ function example() {\n context\n-deleted value\n+added value\n tail',
    };
    const detail = {
      number: 4, title: 'Review me', url: 'https://github.com/o/r/pull/4', state: 'open' as const, draft: false,
      authorLogin: 'ada', headRefName: 'feature', baseRefName: 'main', updatedAt: '2026-07-15T12:00:00Z',
      authoredByViewer: false, reviewRequestedFromViewer: true, unresolvedThreadCount: 1,
      commentCount: 3,
      approvalCount: 1,
      body: '## Description', additions: 4, deletions: 2, changedFiles: 3, mergeability: 'mergeable' as const,
      conversationTruncated: false, reviewThreadsTruncated: false, comments: [comment],
      reviewThreads: [{ id: 'thread-1', path: 'src/app.ts', line: 12, resolved: false, outdated: true, comments: [comment] }],
    };
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(), githubStartDeviceFlow: vi.fn(), githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(), githubConnectPat: vi.fn(), githubConnectCli: vi.fn(), githubDisconnectAccount: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
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
    expect(fixture.nativeElement.querySelector('.body h2')?.textContent).toBe('Description');
    expect(fixture.nativeElement.querySelector('.comment-body strong')?.textContent).toBe('Please');
    expect(fixture.nativeElement.querySelector('.metadata [aria-label="3 comments"]')).toBeTruthy();
    expect(fixture.nativeElement.querySelector('.metadata .branch-route')?.textContent).toContain('feature');
    expect(fixture.nativeElement.querySelector('.metadata .branch-route')?.textContent).toContain('main');
    expect(fixture.nativeElement.querySelector('.state-badge')?.getAttribute('data-state')).toBe('open');
    expect(fixture.nativeElement.querySelector('.mergeability')?.getAttribute('data-mergeability')).toBe('mergeable');
    expect(fixture.nativeElement.querySelector('.avatar')?.textContent.trim()).toBe('G');
    expect(fixture.nativeElement.querySelector('.thread-state')?.classList.contains('outdated')).toBe(true);
    expect(fixture.nativeElement.querySelector('.review-code > header strong')?.textContent).toContain('src/app.ts');
    expect(fixture.nativeElement.querySelector('.review-diff-row[data-kind="deletion"]')?.textContent).toContain('deleted value');
    expect(fixture.nativeElement.querySelector('.review-diff-row[data-kind="addition"]')?.textContent).toContain('added value');
    expect(fixture.nativeElement.querySelectorAll('.review-diff-row[data-kind="context"]')).toHaveLength(2);
    expect(fixture.nativeElement.querySelector('#github-pr-conversation-heading')?.textContent).toContain('1');
    expect(fixture.nativeElement.querySelector('#github-review-threads-heading')?.textContent).toContain('1');
    const open = fixture.nativeElement.querySelector('.open-pr') as HTMLAnchorElement;
    expect(open.href).toBe('https://github.com/o/r/pull/4');
    expect(open.target).toBe('_blank');
    expect(open.getAttribute('aria-label')).toBe('Open pull request #4 on GitHub');
    expect(fixture.nativeElement.textContent).not.toContain('Reply to review thread');
    expect(fixture.nativeElement.textContent).not.toContain('Check out branch');
    expect(fixture.nativeElement.textContent).not.toContain('Checks');
  });
});
