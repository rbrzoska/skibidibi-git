import type { RepositoryBranch } from '../../core/ipc/desktop-ipc';

export interface BranchTreeFolderNode {
  readonly kind: 'folder';
  readonly name: string;
  readonly path: string;
  readonly children: readonly BranchTreeNode[];
  readonly branchCount: number;
}

export interface BranchTreeLeafNode {
  readonly kind: 'branch';
  readonly name: string;
  readonly path: string;
  readonly branch: RepositoryBranch;
}

export type BranchTreeNode = BranchTreeFolderNode | BranchTreeLeafNode;

export interface VisibleBranchTreeFolderNode {
  readonly kind: 'folder';
  readonly name: string;
  readonly path: string;
  readonly depth: number;
  readonly branchCount: number;
  readonly collapsed: boolean;
}

export interface VisibleBranchTreeLeafNode {
  readonly kind: 'branch';
  readonly name: string;
  readonly path: string;
  readonly depth: number;
  readonly parentPath: string | null;
  readonly branch: RepositoryBranch;
}

export type VisibleBranchTreeNode =
  | VisibleBranchTreeFolderNode
  | VisibleBranchTreeLeafNode;

interface MutableFolder {
  readonly name: string;
  readonly path: string;
  readonly folders: Map<string, MutableFolder>;
  readonly branches: BranchTreeLeafNode[];
}

function compareText(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function normalizedSegments(name: string): readonly string[] {
  const segments = name
    .split('/')
    .filter((segment) => segment.length > 0);

  return segments.length > 0 ? segments : ['(unnamed)'];
}

function branchTieBreaker(branch: RepositoryBranch): string {
  return [
    branch.fullName,
    branch.oid,
    branch.kind,
    branch.current ? '1' : '0',
    branch.upstream ?? '',
    String(branch.ahead),
    String(branch.behind),
    branch.upstreamGone ? '1' : '0',
    branch.symbolicTarget ?? '',
  ].join('\u0000');
}

function freezeFolder(folder: MutableFolder): BranchTreeFolderNode {
  const folders = [...folder.folders.values()]
    .sort((left, right) => compareText(left.name, right.name))
    .map(freezeFolder);
  const branches = [...folder.branches].sort((left, right) => {
    const nameComparison = compareText(left.name, right.name);
    return nameComparison !== 0
      ? nameComparison
      : compareText(branchTieBreaker(left.branch), branchTieBreaker(right.branch));
  });

  const children: readonly BranchTreeNode[] = [...folders, ...branches];
  return {
    kind: 'folder',
    name: folder.name,
    path: folder.path,
    children,
    branchCount: children.reduce(
      (count, child) => count + (child.kind === 'folder' ? child.branchCount : 1),
      0,
    ),
  };
}

/**
 * Builds one branch hierarchy. Local and remote branch arrays intentionally stay
 * separate so callers can render independent sections and collapse state.
 */
export function buildBranchTree(
  branches: readonly RepositoryBranch[],
): readonly BranchTreeNode[] {
  const root: MutableFolder = {
    name: '',
    path: '',
    folders: new Map(),
    branches: [],
  };

  for (const branch of branches) {
    const segments = normalizedSegments(branch.name);
    const leafName = segments.at(-1) ?? '(unnamed)';
    const folderSegments = segments.slice(0, -1);
    let parent = root;

    for (const folderName of folderSegments) {
      const path = parent.path.length > 0 ? `${parent.path}/${folderName}` : folderName;
      let folder = parent.folders.get(folderName);
      if (!folder) {
        folder = { name: folderName, path, folders: new Map(), branches: [] };
        parent.folders.set(folderName, folder);
      }
      parent = folder;
    }

    parent.branches.push({
      kind: 'branch',
      name: leafName,
      path: segments.join('/'),
      branch,
    });
  }

  const frozenRoot = freezeFolder(root);
  return frozenRoot.children;
}

function branchMatches(branch: BranchTreeLeafNode, normalizedFilter: string): boolean {
  if (normalizedFilter.length === 0) {
    return true;
  }

  return (
    branch.path.toLowerCase().includes(normalizedFilter) ||
    branch.branch.name.toLowerCase().includes(normalizedFilter) ||
    branch.branch.fullName.toLowerCase().includes(normalizedFilter)
  );
}

function folderHasMatch(folder: BranchTreeFolderNode, normalizedFilter: string): boolean {
  return folder.children.some((child) =>
    child.kind === 'folder'
      ? folderHasMatch(child, normalizedFilter)
      : branchMatches(child, normalizedFilter),
  );
}

/**
 * Flattens a hierarchy for a simple Angular `@for`. Filtering reveals matching
 * descendants even when their folders are normally collapsed and retains every
 * ancestor needed to understand the path.
 */
export function visibleBranchTree(
  tree: readonly BranchTreeNode[],
  collapsedFolders: ReadonlySet<string>,
  filter = '',
): readonly VisibleBranchTreeNode[] {
  const normalizedFilter = filter.trim().toLowerCase();
  const filtering = normalizedFilter.length > 0;
  const result: VisibleBranchTreeNode[] = [];

  function append(nodes: readonly BranchTreeNode[], depth: number, parentPath: string | null): void {
    for (const node of nodes) {
      if (node.kind === 'branch') {
        if (branchMatches(node, normalizedFilter)) {
          result.push({ ...node, depth, parentPath });
        }
        continue;
      }

      if (filtering && !folderHasMatch(node, normalizedFilter)) {
        continue;
      }

      const collapsed = !filtering && collapsedFolders.has(node.path);
      result.push({
        kind: 'folder',
        name: node.name,
        path: node.path,
        depth,
        branchCount: node.branchCount,
        collapsed,
      });

      if (!collapsed) {
        append(node.children, depth + 1, node.path);
      }
    }
  }

  append(tree, 0, null);
  return result;
}
