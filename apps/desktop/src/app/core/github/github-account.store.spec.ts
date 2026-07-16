import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import { GitHubAccountStore } from './github-account.store';
import { GITHUB_BRIDGE, type GitHubAccount, type GitHubBridge } from './github-bridge';

const account: GitHubAccount = { id: 'account-1', login: 'ada', host: 'github.com', avatarUrl: null, state: 'connected' };

describe('GitHubAccountStore', () => {
  it('loads, connects, and disconnects accounts through the narrow bridge', async () => {
    const bridge: GitHubBridge = {
      githubListAccounts: vi.fn().mockResolvedValue([]),
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
});
