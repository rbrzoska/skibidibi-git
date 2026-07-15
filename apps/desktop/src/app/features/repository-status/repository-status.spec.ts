import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';

import {
  DESKTOP_IPC,
  type DesktopIpcClient,
  type RepositoryStatusResponse,
} from '../../core/ipc/desktop-ipc';
import { RepositoryStatusStore } from './repository-status';

const status: RepositoryStatusResponse = {
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
});
