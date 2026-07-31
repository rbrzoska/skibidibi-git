import { describe, expect, it, vi } from 'vitest';

import {
  navigationFavoritesStorageKey,
  readNavigationFavorites,
  writeNavigationFavorites,
  type NavigationFavoritesStorage,
} from './navigation-favorites-state';

function storage(initial: Record<string, string> = {}): NavigationFavoritesStorage {
  const values = new Map(Object.entries(initial));
  return {
    getItem: vi.fn((key) => values.get(key) ?? null),
    setItem: vi.fn((key, value) => values.set(key, value)),
  };
}

describe('navigation favorites persistence', () => {
  it('reads valid unique favorites and removes references that disappeared', () => {
    const repositoryId = 'repo/one';
    const target = storage({
      [navigationFavoritesStorageKey(repositoryId)]: JSON.stringify({
        branches: ['refs/heads/keep', 'refs/heads/gone', 'refs/heads/keep'],
        worktrees: ['/tmp/keep', '/tmp/gone'],
      }),
    });

    expect(readNavigationFavorites(
      target,
      repositoryId,
      new Set(['refs/heads/keep']),
      new Set(['/tmp/keep']),
    )).toEqual({
      branches: ['refs/heads/keep'],
      worktrees: ['/tmp/keep'],
    });
  });

  it('returns an empty state for malformed data', () => {
    const repositoryId = 'repo';
    const target = storage({
      [navigationFavoritesStorageKey(repositoryId)]: '{bad json',
    });

    expect(readNavigationFavorites(target, repositoryId)).toEqual({ branches: [], worktrees: [] });
  });

  it('writes stable deduplicated arrays without surfacing storage failures', () => {
    const target = storage();
    writeNavigationFavorites(target, 'repo', {
      branches: ['refs/heads/z', 'refs/heads/a', 'refs/heads/z'],
      worktrees: ['/z', '/a'],
    });

    expect(target.setItem).toHaveBeenCalledWith(
      navigationFavoritesStorageKey('repo'),
      JSON.stringify({
        branches: ['refs/heads/a', 'refs/heads/z'],
        worktrees: ['/a', '/z'],
      }),
    );
    expect(() => writeNavigationFavorites({
      getItem: () => null,
      setItem: () => { throw new Error('blocked'); },
    }, 'repo', { branches: [], worktrees: [] })).not.toThrow();
  });
});
