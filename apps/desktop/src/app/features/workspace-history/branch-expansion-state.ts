import type { BranchTreeNode } from './branch-tree';

export type BranchExpansionScope = 'local' | 'remote';

export interface BranchExpansionStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const STORAGE_PREFIX = 'skibidibi-git:branch-expansion:v1';

/**
 * Resolves browser storage without assuming that it is available. Accessing
 * localStorage can itself throw in sandboxed/private browser contexts.
 */
export function browserBranchExpansionStorage(): BranchExpansionStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function branchExpansionStorageKey(
  repositoryId: string,
  scope: BranchExpansionScope,
): string {
  return `${STORAGE_PREFIX}:${encodeURIComponent(repositoryId)}:${scope}`;
}

/** Collects every folder path currently present in a branch tree. */
export function branchFolderPaths(tree: readonly BranchTreeNode[]): ReadonlySet<string> {
  const paths = new Set<string>();

  function visit(nodes: readonly BranchTreeNode[]): void {
    for (const node of nodes) {
      if (node.kind !== 'folder') {
        continue;
      }

      paths.add(node.path);
      visit(node.children);
    }
  }

  visit(tree);
  return paths;
}

function sameSet(left: ReadonlySet<string>, right: ReadonlySet<string>): boolean {
  return left.size === right.size && [...left].every((path) => right.has(path));
}

function normalizedPaths(
  paths: Iterable<string>,
  availablePaths: ReadonlySet<string>,
): ReadonlySet<string> {
  return new Set([...paths].filter((path) => path.length > 0 && availablePaths.has(path)).sort());
}

function decodePaths(value: string): ReadonlySet<string> | null {
  try {
    const decoded: unknown = JSON.parse(value);
    if (!Array.isArray(decoded) || !decoded.every((path) => typeof path === 'string')) {
      return null;
    }

    return new Set(decoded);
  } catch {
    return null;
  }
}

/**
 * Small persistence boundary for expanded branch folders. Its empty state means
 * every folder is collapsed, so a repository that has never been opened needs
 * no stored record.
 */
export class BranchExpansionState {
  constructor(
    private readonly storage: BranchExpansionStorage | null = browserBranchExpansionStorage(),
  ) {}

  read(
    repositoryId: string,
    scope: BranchExpansionScope,
    availablePaths: ReadonlySet<string>,
  ): ReadonlySet<string> {
    const key = branchExpansionStorageKey(repositoryId, scope);
    let serialized: string | null;

    try {
      serialized = this.storage?.getItem(key) ?? null;
    } catch {
      return new Set();
    }

    if (serialized === null) {
      return new Set();
    }

    const decoded = decodePaths(serialized);
    if (decoded === null) {
      this.removeSafely(key);
      return new Set();
    }

    const pruned = normalizedPaths(decoded, availablePaths);
    if (!sameSet(decoded, pruned)) {
      this.persistSafely(key, pruned);
    }

    return pruned;
  }

  write(
    repositoryId: string,
    scope: BranchExpansionScope,
    expandedPaths: Iterable<string>,
    availablePaths: ReadonlySet<string>,
  ): ReadonlySet<string> {
    const paths = normalizedPaths(expandedPaths, availablePaths);
    this.persistSafely(branchExpansionStorageKey(repositoryId, scope), paths);
    return paths;
  }

  toggle(
    repositoryId: string,
    scope: BranchExpansionScope,
    path: string,
    expandedPaths: ReadonlySet<string>,
    availablePaths: ReadonlySet<string>,
  ): ReadonlySet<string> {
    const next = new Set(expandedPaths);
    if (next.has(path)) {
      next.delete(path);
    } else if (availablePaths.has(path)) {
      next.add(path);
    }

    return this.write(repositoryId, scope, next, availablePaths);
  }

  private persistSafely(key: string, paths: ReadonlySet<string>): void {
    if (paths.size === 0) {
      this.removeSafely(key);
      return;
    }

    try {
      this.storage?.setItem(key, JSON.stringify([...paths].sort()));
    } catch {
      // Persistence is best-effort. The caller still receives usable UI state.
    }
  }

  private removeSafely(key: string): void {
    try {
      this.storage?.removeItem(key);
    } catch {
      // Storage cleanup must never make the repository view unavailable.
    }
  }
}
