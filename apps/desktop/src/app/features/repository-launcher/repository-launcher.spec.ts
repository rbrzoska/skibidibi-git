import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter, Router } from '@angular/router';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubBridge } from '../../core/github';
import { DESKTOP_IPC, type DesktopIpcClient } from '../../core/ipc/desktop-ipc';
import { RepositoryStatusStore } from '../repository-status/repository-status';
import { RepositoryLauncher } from './repository-launcher';

const rememberedRepository = {
  id: 'skibidibi-git',
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
  lastOpenedAt: Math.floor(Date.now() / 1000),
  createdAt: 1,
  updatedAt: 10,
};

describe('RepositoryLauncher', () => {
  let fixture: ComponentFixture<RepositoryLauncher>;
  let ipcInvoke: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    ipcInvoke = vi.fn().mockImplementation((command: string) => {
      if (command === 'list_remembered_repositories') {
        return Promise.resolve([rememberedRepository]);
      }
      if (command === 'remember_repository') {
        return Promise.resolve(rememberedRepository);
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
    expect(rows[0].textContent).toContain('GitHub connected');
  });

  it('opens a remembered repository in its workspace', async () => {
    const router = TestBed.inject(Router);
    const navigate = vi.spyOn(router, 'navigate').mockResolvedValue(true);
    const row = fixture.nativeElement.querySelector('.repository-row') as HTMLButtonElement;

    row.click();
    await fixture.whenStable();

    expect(TestBed.inject(RepositoryStatusStore).repositoryPath()).toContain('skibidibi-git');
    expect(navigate).toHaveBeenCalledWith(['/workspace', 'skibidibi-git', 'history']);
  });

  it('remembers a newly selected repository exactly once', async () => {
    const router = TestBed.inject(Router);
    vi.spyOn(router, 'navigate').mockResolvedValue(true);
    TestBed.inject(RepositoryStatusStore).setRepositoryPath('/work/new-repository');
    fixture.detectChanges();

    const open = fixture.nativeElement.querySelector('.open-workspace') as HTMLButtonElement;
    open.click();
    await fixture.whenStable();

    expect(
      ipcInvoke.mock.calls.filter(([command]) => command === 'remember_repository'),
    ).toHaveLength(1);
  });
});
