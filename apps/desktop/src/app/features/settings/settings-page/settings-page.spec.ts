import { signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { GITHUB_BRIDGE, GitHubAccountStore, type GitHubBridge } from '../../../core/github';
import { DESKTOP_IPC } from '../../../core/ipc/desktop-ipc';
import { RepositoryCatalog } from '../../../core/repositories/repository-catalog';
import { globalWorkspaceRefreshStorageKey } from '../../workspace-history/workspace-refresh-policy';
import { SettingsPage } from './settings-page';

describe('SettingsPage', () => {
  let component: SettingsPage;
  let fixture: ComponentFixture<SettingsPage>;
  const catalog = {
    repositories: signal([
      {
        id: 'repo-1',
        name: 'skibidibi-git',
        path: '/projects/skibidibi-git',
        provider: 'github',
        transport: 'ssh',
        hostedIdentity: null,
        remote: 'github.com/rbrzoska/skibidibi-git',
        integration: 'connected',
        availability: 'available',
        pinned: false,
        lastOpenedLabel: '2 min ago',
      },
      {
        id: 'repo-2',
        name: 'archive-repository',
        path: '/projects/archive-repository',
        provider: 'local',
        transport: 'none',
        hostedIdentity: null,
        remote: null,
        integration: 'local-only',
        availability: 'available',
        pinned: false,
        lastOpenedLabel: '30 days ago',
      },
    ]),
    state: signal({ kind: 'ready' as const }),
    load: vi.fn().mockResolvedValue(undefined),
  };
  const githubBridge: GitHubBridge = {
    githubListAccounts: vi.fn().mockResolvedValue([]),
    githubListRepositories: vi.fn().mockResolvedValue({ repositories: [], nextCursor: null }),
    githubConnectPat: vi.fn(),
    githubDisconnectAccount: vi.fn(),
    githubListPullRequests: vi.fn(),
    githubPullRequestDetail: vi.fn(),
  };
  const maintenanceInvoke = vi.fn();
  const maintenanceStats = {
    repositoryBytes: 12_582_912,
    gitBytes: 2_097_152,
    worktreeBytes: 10_485_760,
    scannedAt: 1_768_564_800,
    repositoryLastCommitAt: '2025-10-01T09:00:00+02:00',
    worktrees: [
      {
        path: '/projects/skibidibi-git/worktrees/recent',
        branch: 'feature/recent',
        bytes: 524_288,
        lastCommitAt: '2026-01-10T09:00:00+01:00',
        lastOpenedAt: 1_768_500_000,
      },
      {
        path: '/projects/skibidibi-git/worktrees/old',
        branch: 'feature/old',
        bytes: 1_572_864,
        lastCommitAt: '2024-10-01T09:00:00+02:00',
        lastOpenedAt: 1_768_564_000,
      },
      {
        path: '/projects/skibidibi-git/worktrees/unknown',
        branch: null,
        bytes: 256,
        lastCommitAt: null,
        lastOpenedAt: null,
      },
    ],
  };

  beforeEach(async () => {
    maintenanceInvoke.mockReset().mockResolvedValue(maintenanceStats);
    await TestBed.configureTestingModule({
      imports: [SettingsPage],
      providers: [
        GitHubAccountStore,
        { provide: GITHUB_BRIDGE, useValue: githubBridge },
        { provide: RepositoryCatalog, useValue: catalog },
        { provide: DESKTOP_IPC, useValue: { invoke: maintenanceInvoke } },
        provideRouter([]),
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(SettingsPage);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });

  it('renders account, refresh, cloning, and repository maintenance sections', () => {
    fixture.detectChanges();
    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';

    expect(text).toContain('GitHub accounts');
    expect(text).toContain('Refresh defaults');
    expect(text).toContain('Cloning preferences');
    expect(text).toContain('Repository maintenance');
    expect(text).toContain('skibidibi-git');
    expect(text).toContain('Available');
    expect(text).toContain('on-demand scan');
    expect(text).toContain('Back to repositories');
    expect((fixture.nativeElement as HTMLElement).querySelector<HTMLAnchorElement>('.back-link')?.getAttribute('href'))
      .toBe('/repositories');
  });

  it('persists global refresh defaults without creating a repository override', () => {
    fixture.detectChanges();
    const labels = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLLabelElement>('.setting-row')];
    const autoFetch = labels.find((label) => label.textContent?.includes('Auto fetch'))
      ?.querySelector<HTMLInputElement>('input');

    expect(autoFetch).toBeTruthy();
    autoFetch?.click();
    fixture.detectChanges();

    expect(localStorage.getItem(globalWorkspaceRefreshStorageKey('autoFetch'))).toBe('true');
    expect(localStorage.getItem('skibidibi-git.workspace.auto-fetch.repo-1')).toBeNull();
  });

  it('scans and renders repository and worktree maintenance statistics on demand', async () => {
    fixture.detectChanges();
    const scanButton = (fixture.nativeElement as HTMLElement)
      .querySelector<HTMLButtonElement>('.scan-button');

    scanButton?.click();
    fixture.detectChanges();
    expect(scanButton?.textContent).toContain('Scanning');
    await fixture.whenStable();
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(maintenanceInvoke).toHaveBeenCalledWith('repository_maintenance_stats', {
      repositoryId: 'repo-1',
    });
    expect(text).toContain('12 MiB');
    expect(text).toContain('2 MiB');
    expect(text).toContain('feature/old');
    expect(text).toContain('1.5 MiB');
    expect(text).toContain('Very old');
    expect(text).toContain('Commit age unknown');
    expect(text.indexOf('feature/old')).toBeLessThan(text.indexOf('feature/recent'));
    expect(text.indexOf('feature/recent')).toBeLessThan(text.indexOf('Detached HEAD'));
    expect(text).toContain('Last commit');
    expect(text).toContain('recorded by app');
    expect((fixture.nativeElement as HTMLElement)
      .querySelector('time[datetime="2024-10-01T07:00:00.000Z"]')).toBeTruthy();
    expect((fixture.nativeElement as HTMLElement)
      .querySelector('.worktree-stats li:last-child time[datetime]')).toBeNull();
  });

  it('keeps scan errors scoped to the selected repository', async () => {
    maintenanceInvoke.mockRejectedValueOnce(new Error('Disk scan denied'));
    fixture.detectChanges();

    (fixture.nativeElement as HTMLElement).querySelector<HTMLButtonElement>('.scan-button')?.click();
    await fixture.whenStable();
    fixture.detectChanges();

    expect((fixture.nativeElement as HTMLElement).textContent).toContain('Disk scan denied');
    expect((fixture.nativeElement as HTMLElement).querySelector('[role="alert"]')).toBeTruthy();
  });

  it('scans all repositories sequentially and orders scanned repositories by oldest commit', async () => {
    let activeScans = 0;
    let maximumActiveScans = 0;
    maintenanceInvoke.mockImplementation(async (_command, request: { repositoryId: string }) => {
      activeScans += 1;
      maximumActiveScans = Math.max(maximumActiveScans, activeScans);
      await Promise.resolve();
      activeScans -= 1;
      return {
        ...maintenanceStats,
        repositoryLastCommitAt: request.repositoryId === 'repo-2'
          ? '2020-01-01T00:00:00Z'
          : '2025-01-01T00:00:00Z',
      };
    });
    fixture.detectChanges();

    [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('.compact-button')]
      .find((button) => button.textContent?.includes('Scan all'))?.click();
    await fixture.whenStable();
    fixture.detectChanges();

    const text = (fixture.nativeElement as HTMLElement).textContent ?? '';
    expect(maximumActiveScans).toBe(1);
    expect(maintenanceInvoke).toHaveBeenCalledTimes(2);
    expect(text.indexOf('archive-repository')).toBeLessThan(text.indexOf('skibidibi-git'));
  });

  afterEach(() => {
    for (const setting of ['currentOnly', 'autoFetch', 'liveChanges'] as const) {
      localStorage.removeItem(globalWorkspaceRefreshStorageKey(setting));
    }
  });
});
