import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RepositoryStatusResponse,
} from '../../core/ipc/desktop-ipc';
import { RepositoryStatusStore } from './repository-status';

const status: RepositoryStatusResponse = {
  indexFingerprint: 'fixture-index',
  worktreeFingerprint: 'fixture-worktree',
  branch: {
    oid: 'f00ba4',
    head: 'feature/status',
    upstream: 'origin/feature/status',
    ahead: 2,
    behind: 1,
    detached: false,
    unborn: false,
  },
  entries: [],
};

describe('RepositoryStatusStore', () => {
  it('loads the selected repository directory', async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi
        .fn()
        .mockResolvedValueOnce({ path: '/work/selected-repository' })
        .mockResolvedValueOnce(status),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);

    await store.selectRepositoryDirectory();

    expect(ipc.invoke).toHaveBeenNthCalledWith(1, 'select_repository_directory', {
      initialPath: null,
    });
    expect(ipc.invoke).toHaveBeenNthCalledWith(2, 'repository_status', {
      repositoryPath: '/work/selected-repository',
    });
    expect(store.repositoryPath()).toBe('/work/selected-repository');
    expect(store.state()).toEqual({ kind: 'ready', status });
  });

  it('keeps the current repository when directory selection is cancelled', async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockResolvedValue({ path: null }),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);
    store.setRepositoryPath('/work/current-repository');

    await store.selectRepositoryDirectory();

    expect(ipc.invoke).toHaveBeenCalledOnce();
    expect(store.repositoryPath()).toBe('/work/current-repository');
    expect(store.state()).toEqual({ kind: 'idle' });
  });

  it('passes repositoryPath to the typed IPC command', async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockResolvedValue(status),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);

    store.setRepositoryPath('/work/skibidibi-git');
    await store.refresh();

    expect(ipc.invoke).toHaveBeenCalledWith('repository_status', {
      repositoryPath: '/work/skibidibi-git',
    });
    expect(store.state()).toEqual({ kind: 'ready', status });
  });

  it('maps IPC failures to a recoverable display state', async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockRejectedValue(new Error('bridge unavailable')),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);

    store.setRepositoryPath('/missing/repository');
    await store.refresh();

    expect(store.state()).toEqual({
      kind: 'error',
      message: 'Repository status is unavailable. Check the path and desktop bridge.',
    });
  });

  it('keeps the last ready state when a silent live refresh fails', async () => {
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockRejectedValue(new Error('temporary status failure')),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);
    store.setRepositoryPath('/work/skibidibi-git');
    store.state.set({ kind: 'ready', status });

    await store.refresh({ silent: true });

    expect(store.state()).toEqual({ kind: 'ready', status });
    expect(store.backgroundError()).toContain('may be stale');
  });

  it('does not let a silent refresh replace an active foreground refresh', async () => {
    let resolveSilent: ((value: RepositoryStatusResponse) => void) | undefined;
    const silent = new Promise<RepositoryStatusResponse>((resolve) => {
      resolveSilent = resolve;
    });
    const newest: RepositoryStatusResponse = {
      ...status,
      worktreeFingerprint: 'foreground-result',
    };
    const stale: RepositoryStatusResponse = {
      ...status,
      worktreeFingerprint: 'silent-result',
    };
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockReturnValueOnce(silent).mockResolvedValueOnce(newest),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);
    store.setRepositoryPath('/work/skibidibi-git');
    store.state.set({ kind: 'ready', status });

    const backgroundRefresh = store.refresh({ silent: true });
    await store.refresh();
    resolveSilent?.(stale);
    await backgroundRefresh;

    expect(store.state()).toEqual({ kind: 'ready', status: newest });
  });

  it('keeps the newest result when refresh requests finish out of order', async () => {
    let resolveFirst: ((value: RepositoryStatusResponse) => void) | undefined;
    const first = new Promise<RepositoryStatusResponse>((resolve) => {
      resolveFirst = resolve;
    });
    const newest: RepositoryStatusResponse = {
      ...status,
      branch: { ...status.branch, head: 'newest' },
    };
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockReturnValueOnce(first).mockResolvedValueOnce(newest),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);
    store.setRepositoryPath('/work/skibidibi-git');

    const olderRefresh = store.refresh();
    await store.refresh();
    resolveFirst?.(status);
    await olderRefresh;

    expect(store.state()).toEqual({ kind: 'ready', status: newest });
  });

  it('does not let a pending refresh overwrite an accepted mutation result', async () => {
    let resolveRefresh: ((value: RepositoryStatusResponse) => void) | undefined;
    const pendingRefresh = new Promise<RepositoryStatusResponse>((resolve) => {
      resolveRefresh = resolve;
    });
    const ipc: DesktopIpcClient = {
      invoke: vi.fn().mockReturnValue(pendingRefresh),
    };
    TestBed.configureTestingModule({
      providers: [RepositoryStatusStore, { provide: DESKTOP_IPC, useValue: ipc }],
    });
    const store = TestBed.inject(RepositoryStatusStore);
    store.setRepositoryPath('/work/skibidibi-git');
    const refresh = store.refresh();
    const mutationStatus: RepositoryStatusResponse = {
      ...status,
      indexFingerprint: 'after-mutation',
    };

    store.acceptMutationResult(mutationStatus);
    resolveRefresh?.(status);
    await refresh;

    expect(store.state()).toEqual({ kind: 'ready', status: mutationStatus });
  });
});
