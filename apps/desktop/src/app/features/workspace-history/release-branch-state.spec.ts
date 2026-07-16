import { describe, expect, it, vi } from 'vitest';

import {
  readReleaseBranch,
  releaseBranchStorageKey,
  writeReleaseBranch,
  type ReleaseBranchStorage,
} from './release-branch-state';

function storage(initial: Record<string, string> = {}): ReleaseBranchStorage {
  const values = new Map(Object.entries(initial));
  return {
    getItem: vi.fn((key) => values.get(key) ?? null),
    setItem: vi.fn((key, value) => values.set(key, value)),
    removeItem: vi.fn((key) => values.delete(key)),
  };
}

describe('release branch persistence', () => {
  it('keeps an exact local ref only while it still exists', () => {
    const repositoryId = 'repo/one';
    const key = releaseBranchStorageKey(repositoryId);
    const target = storage({ [key]: 'refs/heads/202607_l' });

    expect(readReleaseBranch(target, repositoryId, new Set(['refs/heads/202607_l']))).toBe(
      'refs/heads/202607_l',
    );
    expect(target.removeItem).not.toHaveBeenCalled();
  });

  it.each(['refs/remotes/origin/release', 'refs/heads/deleted'])('clears unavailable value %s', (value) => {
    const repositoryId = 'repo';
    const key = releaseBranchStorageKey(repositoryId);
    const target = storage({ [key]: value });

    expect(readReleaseBranch(target, repositoryId, new Set())).toBeNull();
    expect(target.removeItem).toHaveBeenCalledWith(key);
  });

  it('writes and clears the exact full ref without throwing on unavailable storage', () => {
    const target = storage();
    writeReleaseBranch(target, 'repo', 'refs/heads/release');
    expect(target.setItem).toHaveBeenCalledWith(
      releaseBranchStorageKey('repo'),
      'refs/heads/release',
    );

    writeReleaseBranch(target, 'repo', null);
    expect(target.removeItem).toHaveBeenCalledWith(releaseBranchStorageKey('repo'));

    expect(() => writeReleaseBranch({
      getItem: () => null,
      setItem: () => { throw new Error('blocked'); },
      removeItem: () => { throw new Error('blocked'); },
    }, 'repo', 'refs/heads/release')).not.toThrow();
  });
});
