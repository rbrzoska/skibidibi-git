import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { DESKTOP_IPC, type DesktopIpcClient } from '../ipc/desktop-ipc';
import { DesktopGitHubBridge } from './desktop-github-bridge';

describe('DesktopGitHubBridge', () => {
  it('maps account metadata without exposing auth metadata to feature stores', async () => {
    const invoke = vi.fn().mockResolvedValue([{
      id: 'github.com:7',
      host: 'github.com',
      login: 'octocat',
      displayName: 'The Octocat',
      avatarUrl: null,
      authKind: 'personalAccessToken',
      scopes: ['repo'],
      state: 'connected',
      lastValidatedAt: 1,
    }]);
    TestBed.configureTestingModule({
      providers: [
        DesktopGitHubBridge,
        { provide: DESKTOP_IPC, useValue: { invoke } as unknown as DesktopIpcClient },
      ],
    });

    const accounts = await TestBed.inject(DesktopGitHubBridge).githubListAccounts();

    expect(accounts).toEqual([{
      id: 'github.com:7',
      host: 'github.com',
      login: 'octocat',
      avatarUrl: null,
      state: 'connected',
    }]);
    expect(invoke).toHaveBeenCalledWith('github_list_accounts', {});
  });

  it('passes a PAT only to the connect command', async () => {
    const invoke = vi.fn().mockResolvedValue({
      id: 'github.com:7',
      host: 'github.com',
      login: 'octocat',
      avatarUrl: null,
      state: 'connected',
    });
    TestBed.configureTestingModule({
      providers: [
        DesktopGitHubBridge,
        { provide: DESKTOP_IPC, useValue: { invoke } as unknown as DesktopIpcClient },
      ],
    });

    await TestBed.inject(DesktopGitHubBridge).githubConnectPat({ token: 'test-token' });

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('github_connect_pat', { token: 'test-token' });
  });

  it('lists authenticated repositories through the account-scoped IPC contract', async () => {
    const page = {
      repositories: [{
        id: '9007199254740993',
        owner: 'octocat',
        name: 'widget',
        fullName: 'octocat/widget',
        private: true,
        updatedAt: '2026-07-16T08:30:00Z',
        httpsCloneUrl: 'https://github.com/octocat/widget.git',
        sshCloneUrl: 'git@github.com:octocat/widget.git',
      }],
      nextCursor: 'page:2',
    };
    const invoke = vi.fn().mockResolvedValue(page);
    TestBed.configureTestingModule({
      providers: [
        DesktopGitHubBridge,
        { provide: DESKTOP_IPC, useValue: { invoke } as unknown as DesktopIpcClient },
      ],
    });

    const repositories = await TestBed.inject(DesktopGitHubBridge).githubListRepositories({
      accountId: 'github.com:7',
      cursor: 'page:1',
      pageSize: 25,
    });

    expect(repositories).toEqual(page);
    expect(invoke).toHaveBeenCalledWith('github_list_repositories', {
      accountId: 'github.com:7',
      cursor: 'page:1',
      pageSize: 25,
    });
  });
});
