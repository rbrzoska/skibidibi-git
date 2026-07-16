const RELEASE_BRANCH_STORAGE_PREFIX = 'skibidibi-git.workspace.release-branch';

export interface ReleaseBranchStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export function releaseBranchStorageKey(repositoryId: string): string {
  return `${RELEASE_BRANCH_STORAGE_PREFIX}.${encodeURIComponent(repositoryId)}`;
}

export function readReleaseBranch(
  storage: ReleaseBranchStorage,
  repositoryId: string,
  availableLocalRefs: ReadonlySet<string>,
): string | null {
  const key = releaseBranchStorageKey(repositoryId);
  try {
    const value = storage.getItem(key);
    if (value === null) {
      return null;
    }
    if (!value.startsWith('refs/heads/') || !availableLocalRefs.has(value)) {
      storage.removeItem(key);
      return null;
    }
    return value;
  } catch {
    return null;
  }
}

export function writeReleaseBranch(
  storage: ReleaseBranchStorage,
  repositoryId: string,
  fullName: string | null,
): void {
  const key = releaseBranchStorageKey(repositoryId);
  try {
    if (fullName === null) {
      storage.removeItem(key);
    } else if (fullName.startsWith('refs/heads/')) {
      storage.setItem(key, fullName);
    }
  } catch {
    // Local preferences must never block repository navigation.
  }
}
