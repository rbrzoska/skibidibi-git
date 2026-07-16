import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GitHubAccountStore } from './github-account.store';
import { GITHUB_BRIDGE, type GitHubAccount, type GitHubBridge } from './github-bridge';

const account: GitHubAccount = { id: 'account-1', login: 'ada', host: 'github.com', avatarUrl: null, state: 'connected' };

describe('GitHubAccountStore', () => {
  it('loads, connects, and disconnects accounts through the narrow bridge', async () => {
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn().mockResolvedValue([]),
      githubStartDeviceFlow: vi.fn(),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn().mockResolvedValue(account),
      githubDisconnectAccount: vi.fn().mockResolvedValue({ disconnected: true }),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    await store.load();
    expect(store.state()).toEqual({ kind: 'ready', accounts: [] });
    expect(await store.connectPat('  secret-pat  ')).toBe(true);
    expect(bridge.githubConnectPat).toHaveBeenCalledWith({ token: 'secret-pat' });
    expect(store.state()).toEqual({ kind: 'ready', accounts: [account] });

    await store.disconnect(account.id);
    expect(bridge.githubDisconnectAccount).toHaveBeenCalledWith({ accountId: account.id });
    expect(store.state()).toEqual({ kind: 'ready', accounts: [] });
  });

  it('ignores an obsolete account-list response', async () => {
    let resolveFirst!: (accounts: readonly GitHubAccount[]) => void;
    const first = new Promise<readonly GitHubAccount[]>((resolve) => { resolveFirst = resolve; });
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn()
        .mockReturnValueOnce(first)
        .mockResolvedValueOnce([account]),
      githubStartDeviceFlow: vi.fn(),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    const oldLoad = store.load();
    await store.load();
    resolveFirst([]);
    await oldLoad;

    expect(store.state()).toEqual({ kind: 'ready', accounts: [account] });
  });

  it('does not let an initial list response erase a concurrently connected account', async () => {
    let resolveList!: (accounts: readonly GitHubAccount[]) => void;
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn().mockReturnValue(new Promise((resolve) => { resolveList = resolve; })),
      githubStartDeviceFlow: vi.fn(),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn().mockResolvedValue(account),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    const initialLoad = store.load();
    await store.connectPat('secret-pat');
    resolveList([]);
    await initialLoad;

    expect(store.state()).toEqual({ kind: 'ready', accounts: [account] });
  });

  it('does not start an account refresh while a connection mutation is pending', async () => {
    let resolveConnect!: (account: GitHubAccount) => void;
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn().mockResolvedValue([]),
      githubStartDeviceFlow: vi.fn(),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn().mockReturnValue(new Promise((resolve) => { resolveConnect = resolve; })),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    const connect = store.connectPat('secret-pat');
    await store.load();
    resolveConnect(account);
    await connect;

    expect(bridge.githubListAccounts).not.toHaveBeenCalled();
    expect(store.state()).toEqual({ kind: 'ready', accounts: [account] });
  });

  it('polls a device flow at the backend-provided cadence and stores the authorized account', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-07-16T10:00:00Z'));
    const nowSeconds = Date.now() / 1_000;
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(),
      githubStartDeviceFlow: vi.fn().mockResolvedValue({
        flowId: 'flow-1',
        userCode: 'ABCD-EFGH',
        verificationUri: 'https://github.com/login/device',
        expiresAt: nowSeconds + 900,
        intervalSeconds: 5,
      }),
      githubPollDeviceFlow: vi.fn()
        .mockResolvedValueOnce({ state: 'pending', nextPollAt: nowSeconds + 10, account: null })
        .mockResolvedValueOnce({ state: 'authorized', nextPollAt: null, account }),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    await store.startDeviceFlow();
    expect(store.deviceFlow().kind).toBe('waiting');
    await vi.advanceTimersByTimeAsync(5_000);
    expect(bridge.githubPollDeviceFlow).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(4_999);
    expect(bridge.githubPollDeviceFlow).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);

    expect(bridge.githubPollDeviceFlow).toHaveBeenCalledTimes(2);
    expect(store.deviceFlow()).toEqual({ kind: 'idle' });
    expect(store.state()).toEqual({ kind: 'ready', accounts: [account] });
    vi.useRealTimers();
  });

  it('cancels the active device flow and stops polling', async () => {
    vi.useFakeTimers();
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(),
      githubStartDeviceFlow: vi.fn().mockResolvedValue({
        flowId: 'flow-1',
        userCode: 'ABCD-EFGH',
        verificationUri: 'https://github.com/login/device',
        expiresAt: Date.now() / 1_000 + 900,
        intervalSeconds: 5,
      }),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn().mockResolvedValue({ cancelled: true }),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    await store.startDeviceFlow();
    await store.cancelDeviceFlow();
    await vi.advanceTimersByTimeAsync(10_000);

    expect(bridge.githubCancelDeviceFlow).toHaveBeenCalledWith({ flowId: 'flow-1' });
    expect(bridge.githubPollDeviceFlow).not.toHaveBeenCalled();
    expect(store.deviceFlow()).toEqual({ kind: 'idle' });
    vi.useRealTimers();
  });

  it('opens only the active native Device Flow session', async () => {
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(),
      githubStartDeviceFlow: vi.fn().mockResolvedValue({
        flowId: 'flow-1',
        userCode: 'ABCD-EFGH',
        verificationUri: 'https://github.com/login/device',
        expiresAt: Date.now() / 1_000 + 900,
        intervalSeconds: 60,
      }),
      githubPollDeviceFlow: vi.fn(),
      githubCancelDeviceFlow: vi.fn(),
      githubOpenDeviceVerification: vi.fn().mockResolvedValue(undefined),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    await store.openDeviceVerification();
    expect(bridge.githubOpenDeviceVerification).not.toHaveBeenCalled();
    await store.startDeviceFlow();
    await store.openDeviceVerification();

    expect(bridge.githubOpenDeviceVerification).toHaveBeenCalledWith({ flowId: 'flow-1' });
    await store.cancelDeviceFlow();
  });

  it('cancels the native session when polling fails', async () => {
    vi.useFakeTimers();
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn(),
      githubStartDeviceFlow: vi.fn().mockResolvedValue({
        flowId: 'flow-failed',
        userCode: 'ABCD-EFGH',
        verificationUri: 'https://github.com/login/device',
        expiresAt: Date.now() / 1_000 + 900,
        intervalSeconds: 1,
      }),
      githubPollDeviceFlow: vi.fn().mockRejectedValue(new Error('offline')),
      githubCancelDeviceFlow: vi.fn().mockResolvedValue({ cancelled: true }),
      githubOpenDeviceVerification: vi.fn(),
      githubConnectPat: vi.fn(),
      githubDisconnectAccount: vi.fn(),
      githubListRepositories: vi.fn(),
      githubListPullRequests: vi.fn(),
      githubPullRequestDetail: vi.fn(),
    };
    TestBed.configureTestingModule({ providers: [GitHubAccountStore, { provide: GITHUB_BRIDGE, useValue: bridge }] });
    const store = TestBed.inject(GitHubAccountStore);

    await store.startDeviceFlow();
    await vi.advanceTimersByTimeAsync(1_000);

    expect(bridge.githubCancelDeviceFlow).toHaveBeenCalledWith({ flowId: 'flow-failed' });
    expect(store.deviceFlow()).toEqual({ kind: 'idle' });
    vi.useRealTimers();
  });
});
