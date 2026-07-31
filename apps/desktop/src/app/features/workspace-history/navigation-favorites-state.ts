const NAVIGATION_FAVORITES_STORAGE_PREFIX = 'skibidibi-git.workspace.navigation-favorites.v1';

export interface NavigationFavorites {
  readonly branches: readonly string[];
  readonly worktrees: readonly string[];
}

export interface NavigationFavoritesStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function navigationFavoritesStorageKey(repositoryId: string): string {
  return `${NAVIGATION_FAVORITES_STORAGE_PREFIX}.${encodeURIComponent(repositoryId)}`;
}

export function readNavigationFavorites(
  storage: NavigationFavoritesStorage,
  repositoryId: string,
  availableBranches?: ReadonlySet<string>,
  availableWorktrees?: ReadonlySet<string>,
): NavigationFavorites {
  try {
    const serialized = storage.getItem(navigationFavoritesStorageKey(repositoryId));
    if (serialized === null) {
      return { branches: [], worktrees: [] };
    }
    const parsed: unknown = JSON.parse(serialized);
    if (typeof parsed !== 'object' || parsed === null) {
      return { branches: [], worktrees: [] };
    }
    const record = parsed as Record<string, unknown>;
    return {
      branches: validStrings(record['branches']).filter(
        (branch) => availableBranches === undefined || availableBranches.has(branch),
      ),
      worktrees: validStrings(record['worktrees']).filter(
        (worktree) => availableWorktrees === undefined || availableWorktrees.has(worktree),
      ),
    };
  } catch {
    return { branches: [], worktrees: [] };
  }
}

export function writeNavigationFavorites(
  storage: NavigationFavoritesStorage,
  repositoryId: string,
  favorites: NavigationFavorites,
): void {
  try {
    storage.setItem(
      navigationFavoritesStorageKey(repositoryId),
      JSON.stringify({
        branches: [...new Set(favorites.branches)].sort(),
        worktrees: [...new Set(favorites.worktrees)].sort(),
      }),
    );
  } catch {
    // Navigation preferences must never block repository operations.
  }
}

function validStrings(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return [...new Set(value.filter((item): item is string => typeof item === 'string' && item.length > 0))];
}
