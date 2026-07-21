import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubBridge } from '../../../core/github';
import { DESKTOP_IPC, type DesktopIpcClient } from '../../../core/ipc/desktop-ipc';
import { CloneRepositoryDialog, suggestDirectoryName } from './clone-repository-dialog';

const remembered = {
  id: 'cloned', canonicalPath: '/projects/demo', displayName: 'demo', provider: 'github' as const,
  transport: 'ssh' as const, hostedIdentity: { host: 'github.com', owner: 'owner', name: 'demo' },
  availability: 'available' as const,
  gitHealth: { state: 'healthy' as const, issue: null, checkedAt: 1 },
  githubHealth: { state: 'unknown' as const, issue: null, checkedAt: null },
  pinned: false, openCount: 1, lastOpenedAt: 1, createdAt: 1, updatedAt: 1,
};

describe('CloneRepositoryDialog', () => {
  let fixture: ComponentFixture<CloneRepositoryDialog>;
  let invoke: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    invoke = vi.fn().mockImplementation((command: string) => {
      if (command === 'select_clone_parent_directory') return Promise.resolve({ path: '/projects' });
      if (command === 'clone_repository') return Promise.resolve(remembered);
      if (command === 'list_remembered_repositories') return Promise.resolve([]);
      return Promise.reject(new Error(`Unexpected ${command}`));
    });
    const ipc: DesktopIpcClient = { invoke: invoke as DesktopIpcClient['invoke'] };
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
      imports: [CloneRepositoryDialog],
      providers: [
        GitHubAccountStore,
        { provide: DESKTOP_IPC, useValue: ipc },
        { provide: GITHUB_BRIDGE, useValue: github },
      ],
    }).compileComponents();
    fixture = TestBed.createComponent(CloneRepositoryDialog);
    fixture.detectChanges();
  });

  it('derives a safe default directory label from common clone URLs', () => {
    expect(suggestDirectoryName('git@github.com:owner/demo.git')).toBe('demo');
    expect(suggestDirectoryName('https://github.com/owner/demo.git')).toBe('demo');
  });

  it('selects a parent and submits the exact URL clone request', async () => {
    const buttons = fixture.nativeElement.querySelectorAll('button') as NodeListOf<HTMLButtonElement>;
    [...buttons].find((button) => button.textContent?.includes('Browse'))?.click();
    await fixture.whenStable();
    const inputs = fixture.nativeElement.querySelectorAll('input[type="text"]') as NodeListOf<HTMLInputElement>;
    inputs[0].value = 'git@github.com:owner/demo.git';
    inputs[0].dispatchEvent(new Event('input'));
    fixture.detectChanges();
    [...buttons].find((button) => button.textContent?.includes('Clone repository'))?.click();
    await fixture.whenStable();

    expect(invoke).toHaveBeenCalledWith('clone_repository', {
      sourceUrl: 'git@github.com:owner/demo.git',
      destinationParent: '/projects',
      directoryName: 'demo',
    });
  });
});
