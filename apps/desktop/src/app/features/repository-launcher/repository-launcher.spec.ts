import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter, Router } from '@angular/router';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubBridge } from '../../core/github';
import { DESKTOP_IPC, type DesktopIpcClient } from '../../core/ipc/desktop-ipc';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import { RepositoryLauncher } from './repository-launcher';

const rememberedRepository = {
  id: 'skibidibi-git',
  repositoryGroupId: 'skibidibi-group',
  worktreeRole: 'main' as const,
  canonicalPath: '/work/skibidibi-git',
  displayName: 'skibidibi-git',
  provider: 'github' as const,
  transport: 'ssh' as const,
  hostedIdentity: { host: 'github.com', owner: 'rbrzoska', name: 'skibidibi-git' },
  availability: 'available' as const,
  gitHealth: { state: 'healthy' as const, issue: null, checkedAt: 10 },
  githubHealth: { state: 'healthy' as const, issue: null, checkedAt: 10 },
  pinned: false,
  openCount: 1,
  lastOpenedAt: 100,
  createdAt: 1,
  updatedAt: 10,
};

const linkedWorktree = {
  ...rememberedRepository,
  id: 'skibidibi-feature',
  worktreeRole: 'linked' as const,
  canonicalPath: '/work/skibidibi-feature',
  displayName: 'skibidibi-feature',
  lastOpenedAt: 200,
};

describe('RepositoryLauncher', () => {
  let fixture: ComponentFixture<RepositoryLauncher>;
  let ipcInvoke: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    ipcInvoke = vi.fn().mockImplementation((command: string, request?: { repositoryPath?: string }) => {
      if (command === 'list_remembered_repositories') {
        return Promise.resolve([linkedWorktree, rememberedRepository]);
      }
      if (command === 'remember_repository') {
        if (request?.repositoryPath === '/work/new-repository') {
          return Promise.resolve({
            ...rememberedRepository,
            id: 'new-repository',
            repositoryGroupId: 'new-group',
            canonicalPath: '/work/new-repository',
            displayName: 'new-repository',
          });
        }
        return Promise.resolve(
          request?.repositoryPath === linkedWorktree.canonicalPath
            ? linkedWorktree
            : rememberedRepository,
        );
      }
      if (command === 'select_repository_directory') {
        return Promise.resolve({ path: '/work/new-repository' });
      }
      return Promise.resolve({
        branch: { oid: 'abc', head: 'main', upstream: 'origin/main', ahead: 0, behind: 0, detached: false, unborn: false },
        entries: [],
      });
    });
    const ipc: DesktopIpcClient = {
      invoke: ipcInvoke as DesktopIpcClient['invoke'],
    };
    const github: GitHubBridge = {
      githubListAccounts: vi.fn().mockResolvedValue([]),
      githubStartDeviceFlow: vi.fn(),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn(),
      githubConnectCli: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn().mockResolvedValue({ repositories: [], nextCursor: null }),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    await TestBed.configureTestingModule({
      imports: [RepositoryLauncher],
      providers: [
        provideRouter([]),
        GitHubAccountStore,
        { provide: DESKTOP_IPC, useValue: ipc },
        { provide: GITHUB_BRIDGE, useValue: github },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(RepositoryLauncher);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
  });

  it('renders clickable remembered repositories', () => {
    const rows = fixture.nativeElement.querySelectorAll('.repository-row') as NodeListOf<HTMLButtonElement>;

    expect(rows).toHaveLength(1);
    expect(rows[0].textContent).toContain('skibidibi-git');
    expect(rows[0].textContent).toContain('PRs connected');
    expect(rows[0].textContent).toContain('2 worktrees');
    expect(rows[0].textContent).toContain('/work/skibidibi-feature');
    expect(fixture.nativeElement.querySelectorAll('.worktree-row')).toHaveLength(2);
    expect(fixture.nativeElement.textContent).toContain('/work/skibidibi-git');
    expect(fixture.nativeElement.textContent).toContain('/work/skibidibi-feature');
  });

  it('opens the most recently used available worktree from the group header', async () => {
    const router = TestBed.inject(Router);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);
    const row = fixture.nativeElement.querySelector('.repository-row') as HTMLButtonElement;

    row.click();
    await fixture.whenStable();

    expect(TestBed.inject(RepositoryStatusStore).repositoryPath()).toBe('/work/skibidibi-feature');
    expect(navigate).toHaveBeenCalledWith(['/workspace', 'skibidibi-feature', 'history']);
  });

  it('opens the specific repository selected from the worktree list', async () => {
    const router = TestBed.inject(Router);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);
    const worktreeRows = fixture.nativeElement.querySelectorAll(
      '.worktree-row',
    ) as NodeListOf<HTMLButtonElement>;

    worktreeRows[0].click();
    await fixture.whenStable();

    expect(ipcInvoke).toHaveBeenCalledWith('remember_repository', {
      repositoryPath: '/work/skibidibi-git',
    });
    expect(navigate).toHaveBeenCalledWith(['/workspace', 'skibidibi-git', 'history']);
  });

  it('remembers a newly selected repository exactly once', async () => {
    const router = TestBed.inject(Router);
    vi.spyOn(router, 'navigate').mockResolvedValue(true);
    const addLocal = [...fixture.nativeElement.querySelectorAll('button')]
      .find((button: HTMLButtonElement) => button.textContent?.includes('Add local')) as HTMLButtonElement;
    addLocal.click();
    await fixture.whenStable();

    expect(
      ipcInvoke.mock.calls.filter(([command]) => command === 'remember_repository'),
    ).toHaveLength(1);
    expect(TestBed.inject(RepositoryStatusStore).repositoryPath()).toBe('/work/new-repository');
  });
});
