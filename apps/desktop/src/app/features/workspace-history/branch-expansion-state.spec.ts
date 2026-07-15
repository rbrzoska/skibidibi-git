import { describe, expect, it } from 'vitest';

import type { RepositoryBranch } from '../../core/ipc/desktop-ipc';
import { buildBranchTree } from './branch-tree';
import {
  BranchExpansionState,
  branchExpansionStorageKey,
  branchFolderPaths,
  type BranchExpansionStorage,
} from './branch-expansion-state';

class MemoryStorage implements BranchExpansionStorage {
  readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }
}

function branch(name: string): RepositoryBranch {
  return {
    name,
    fullName: `refs/heads/${name}`,
    oid: 'a'.repeat(40),
    kind: 'local',
    current: false,
    upstream: null,
    ahead: 0,
    behind: 0,
    upstreamGone: false,
    symbolicTarget: null,
  };
}

describe('BranchExpansionState', () => {
  it('defaults to every folder collapsed and stores nothing', () => {
    const storage = new MemoryStorage();
    const state = new BranchExpansionState(storage);

    expect(state.read('repo-1', 'local', new Set(['rb']))).toEqual(new Set());
    expect(storage.values.size).toBe(0);
  });

  it('keeps repository and local/remote namespaces independent', () => {
    const storage = new MemoryStorage();
    const state = new BranchExpansionState(storage);
    const available = new Set(['rb']);

    state.write('repo-1', 'local', ['rb'], available);
    state.write('repo-1', 'remote', ['rb'], available);
    state.write('repo-2', 'local', ['rb'], available);

    expect(storage.values.size).toBe(3);
    expect(branchExpansionStorageKey('repo-1', 'local')).not.toBe(
      branchExpansionStorageKey('repo-1', 'remote'),
    );
    expect(branchExpansionStorageKey('repo-1', 'local')).not.toBe(
      branchExpansionStorageKey('repo-2', 'local'),
    );
  });

  it('toggles only existing folders and persists a deterministic array', () => {
    const storage = new MemoryStorage();
    const state = new BranchExpansionState(storage);
    const available = new Set(['rb', 'team', 'team/frontend']);

    let expanded = state.toggle('repo', 'local', 'team/frontend', new Set(), available);
    expanded = state.toggle('repo', 'local', 'rb', expanded, available);
    expanded = state.toggle('repo', 'local', 'missing', expanded, available);

    expect(expanded).toEqual(new Set(['rb', 'team/frontend']));
    expect(storage.getItem(branchExpansionStorageKey('repo', 'local'))).toBe(
      '["rb","team/frontend"]',
    );

    expanded = state.toggle('repo', 'local', 'rb', expanded, available);
    expect(expanded).toEqual(new Set(['team/frontend']));
  });

  it('prunes folders that disappeared from the current tree', () => {
    const storage = new MemoryStorage();
    const key = branchExpansionStorageKey('repo', 'remote');
    storage.setItem(key, JSON.stringify(['origin', 'origin/deleted', 'stale']));
    const state = new BranchExpansionState(storage);

    expect(state.read('repo', 'remote', new Set(['origin', 'origin/current']))).toEqual(
      new Set(['origin']),
    );
    expect(storage.getItem(key)).toBe('["origin"]');

    expect(state.read('repo', 'remote', new Set())).toEqual(new Set());
    expect(storage.getItem(key)).toBeNull();
  });

  it.each(['not-json', '{}', '["rb", 42]', 'null'])(
    'recovers from malformed persisted data: %s',
    (serialized) => {
      const storage = new MemoryStorage();
      const key = branchExpansionStorageKey('repo', 'local');
      storage.setItem(key, serialized);

      expect(new BranchExpansionState(storage).read('repo', 'local', new Set(['rb']))).toEqual(
        new Set(),
      );
      expect(storage.getItem(key)).toBeNull();
    },
  );

  it('never propagates storage read, write, or cleanup failures', () => {
    const throwingStorage: BranchExpansionStorage = {
      getItem: () => {
        throw new Error('blocked read');
      },
      setItem: () => {
        throw new Error('blocked write');
      },
      removeItem: () => {
        throw new Error('blocked cleanup');
      },
    };
    const state = new BranchExpansionState(throwingStorage);

    expect(state.read('repo', 'local', new Set(['rb']))).toEqual(new Set());
    expect(state.write('repo', 'local', ['rb'], new Set(['rb']))).toEqual(new Set(['rb']));
    expect(state.write('repo', 'local', [], new Set(['rb']))).toEqual(new Set());
  });

  it('collects nested folder paths from the current branch tree', () => {
    const tree = buildBranchTree([
      branch('rb/DEV-1'),
      branch('team/frontend/DEV-2'),
      branch('main'),
    ]);

    expect(branchFolderPaths(tree)).toEqual(new Set(['rb', 'team', 'team/frontend']));
  });
});
