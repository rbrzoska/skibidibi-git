import { signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { vi } from 'vitest';

import {
  GITHUB_BRIDGE,
  GithubPullRequestDashboard,
  GitHubAccountStore,
  GitHubRepositoryPullRequestStore,
  type DashboardPullRequest,
} from '../../core/github';
import { RepositoryCatalog } from '../../core/repositories/repository-catalog';
import { CommanderContextStore } from '../../core/commander/commander-context';

import { PullRequestDashboard } from './pull-request-dashboard';

describe('PullRequestDashboard', () => {
  let component: PullRequestDashboard;
  let fixture: ComponentFixture<PullRequestDashboard>;
  let bridge: {
    githubListPullRequests: ReturnType<typeof vi.fn>;
    githubPullRequestDetail: ReturnType<typeof vi.fn>;
    githubPullRequestFiles: ReturnType<typeof vi.fn>;
    githubApprovePullRequest: ReturnType<typeof vi.fn>;
  };

  beforeEach(async () => {
    bridge = {
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
      githubPullRequestFiles: vi.fn().mockResolvedValue({
        files: [{ filename: 'src/app.ts', previousFilename: null, status: 'modified', additions: 2, deletions: 1, changes: 3, patch: '@@ -1 +1 @@\n-old\n+new' }],
        truncated: false,
      }),
      githubApprovePullRequest: vi.fn().mockResolvedValue({ approved: true }),
    };
    await TestBed.configureTestingModule({
      imports: [PullRequestDashboard],
      providers: [
        { provide: GITHUB_BRIDGE, useValue: bridge },
        { provide: GitHubAccountStore, useValue: { state: signal({ kind: 'ready', accounts: [] }), load: vi.fn(), connectCli: vi.fn().mockResolvedValue(false) } },
        { provide: RepositoryCatalog, useValue: { load: vi.fn(), groups: signal([]) } },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(PullRequestDashboard);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });

  it('loads file diffs on demand and approves the selected pull request', async () => {
    const pullRequest: DashboardPullRequest = {
      repositoryId: 'repo-1', repositoryName: 'widget', repositoryFullName: 'acme/widget',
      number: 42, title: 'Review this', url: 'https://github.com/acme/widget/pull/42', state: 'open', draft: false,
      authorLogin: 'contributor', headRefName: 'feature', baseRefName: 'main', updatedAt: '2026-07-21T08:00:00Z',
      authoredByViewer: false, reviewRequestedFromViewer: true, commentCount: 2, approvalCount: 0, unresolvedThreadCount: 0,
    };
    const dashboard = TestBed.inject(GithubPullRequestDashboard);
    const store = fixture.debugElement.injector.get(GitHubRepositoryPullRequestStore);
    dashboard.account.set({ id: 'account-1', login: 'octocat', host: 'github.com', avatarUrl: null, state: 'connected', authKind: 'gitHubCli' });
    bridge.githubPullRequestDetail.mockResolvedValue({
      ...pullRequest,
      body: 'Description',
      additions: 2,
      deletions: 1,
      changedFiles: 1,
      mergeability: 'mergeable',
      comments: [],
      reviewThreads: [],
      conversationTruncated: false,
      reviewThreadsTruncated: false,
    });
    (component as unknown as { select(value: DashboardPullRequest): void }).select(pullRequest);
    await fixture.whenStable();
    fixture.detectChanges();
    expect(TestBed.inject(CommanderContextStore).context().selectedEntity)
      .toBe('pull-request:acme/widget#42|title:Review this');

    (fixture.nativeElement.querySelector('[role="tab"]:nth-child(2)') as HTMLButtonElement).click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.changed-file')?.textContent).toContain('src/app.ts');
    expect(fixture.nativeElement.querySelector('.patch')?.textContent).toContain('+new');
    (fixture.nativeElement.querySelector('.approve') as HTMLButtonElement).click();
    await fixture.whenStable();

    expect(bridge.githubApprovePullRequest).toHaveBeenCalledWith({ repositoryId: 'repo-1', accountId: 'account-1', number: 42 });
    expect(store.approvalState()).toEqual({ kind: 'approved', number: 42 });
  });
});
