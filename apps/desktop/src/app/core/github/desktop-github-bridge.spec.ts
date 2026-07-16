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

  it('uses opaque flow identifiers for the OAuth device flow IPC contract', async () => {
    const invoke = vi.fn()
      .mockResolvedValueOnce({
        flowId: 'flow-1',
        userCode: 'ABCD-EFGH',
        verificationUri: 'https://github.com/login/device',
        expiresAt: 1_800_000_000,
        intervalSeconds: 5,
      })
      .mockResolvedValueOnce({ state: 'pending', nextPollAt: 1_700_000_005, account: null })
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce({ cancelled: true });
    TestBed.configureTestingModule({
      providers: [
        DesktopGitHubBridge,
        { provide: DESKTOP_IPC, useValue: { invoke } as unknown as DesktopIpcClient },
      ],
    });
    const bridge = TestBed.inject(DesktopGitHubBridge);

    const started = await bridge.githubStartDeviceFlow();
    await bridge.githubPollDeviceFlow({ flowId: started.flowId });
    await bridge.githubOpenDeviceVerification({ flowId: started.flowId });
    await bridge.githubCancelDeviceFlow({ flowId: started.flowId });

    expect(invoke.mock.calls).toEqual([
      ['github_start_device_flow', {}],
      ['github_poll_device_flow', { flowId: 'flow-1' }],
      ['github_open_device_verification', { flowId: 'flow-1' }],
      ['github_cancel_device_flow', { flowId: 'flow-1' }],
    ]);
    expect(JSON.stringify(invoke.mock.calls)).not.toContain('deviceCode');
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
