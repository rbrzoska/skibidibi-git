import { describe, expect, it } from 'vitest';

import type { RepositoryBranch } from '../../core/ipc/desktop-ipc';
import { buildBranchTree, visibleBranchTree } from './branch-tree';

function branch(
  name: string,
  overrides: Partial<RepositoryBranch> = {},
): RepositoryBranch {
  return {
    kind: 'local',
    fullName: `refs/heads/${name}`,
    name,
    oid: `oid-${name}`,
    current: false,
    upstream: null,
    ahead: 0,
    behind: 0,
    upstreamGone: false,
    symbolicTarget: null,
    ...overrides,
  };
}

describe('buildBranchTree', () => {
  it('creates deterministic nested folders with folders before branch leaves', () => {
    const tree = buildBranchTree([
      branch('z-last'),
      branch('rb/zebra'),
      branch('alpha'),
      branch('rb/alpha/two'),
      branch('rb/alpha/one'),
      branch('aa/child'),
    ]);

    expect(tree.map((node) => `${node.kind}:${node.name}`)).toEqual([
      'folder:aa',
      'folder:rb',
      'branch:alpha',
      'branch:z-last',
    ]);
    const rb = tree[1];
    expect(rb.kind).toBe('folder');
    if (rb.kind !== 'folder') return;
    expect(rb.branchCount).toBe(3);
    expect(rb.children.map((node) => `${node.kind}:${node.name}`)).toEqual([
      'folder:alpha',
      'branch:zebra',
    ]);
  });

  it('keeps a branch and a same-named folder when paths collide', () => {
    const tree = buildBranchTree([branch('rb'), branch('rb/foo')]);

    expect(tree.map((node) => `${node.kind}:${node.path}`)).toEqual([
      'folder:rb',
      'branch:rb',
    ]);
    const folder = tree[0];
    expect(folder.kind === 'folder' ? folder.children[0] : null).toMatchObject({
      kind: 'branch',
      name: 'foo',
      path: 'rb/foo',
    });
  });

  it('normalizes empty path segments and preserves the original DTO on leaves', () => {
    const odd = branch('/rb//feature/', { fullName: 'refs/heads//rb//feature/' });
    const unnamed = branch('', { fullName: 'refs/heads/' });
    const tree = buildBranchTree([odd, unnamed]);
    const visible = visibleBranchTree(tree, new Set());

    expect(visible.map((node) => `${node.kind}:${node.path}`)).toEqual([
      'folder:rb',
      'branch:rb/feature',
      'branch:(unnamed)',
    ]);
    const oddLeaf = visible.find(
      (node) => node.kind === 'branch' && node.path === 'rb/feature',
    );
    expect(oddLeaf?.kind === 'branch' ? oddLeaf.branch : null).toBe(odd);
  });

  it('preserves Unicode whitespace that is legal in Git ref segments', () => {
    const unicodeName = `feature/\u00a0topic`;
    const tree = buildBranchTree([branch(unicodeName)]);
    const visible = visibleBranchTree(tree, new Set());

    expect(visible.map((node) => node.path)).toEqual(['feature', unicodeName]);
    expect(visible[1]).toMatchObject({ kind: 'branch', name: '\u00a0topic' });
  });

  it('orders without locale-dependent comparison and is independent of input order', () => {
    const inputs = [branch('b'), branch('A'), branch('a'), branch('B')];
    const names = (branches: readonly RepositoryBranch[]) =>
      buildBranchTree(branches).map((node) => node.name);

    expect(names(inputs)).toEqual(['A', 'B', 'a', 'b']);
    expect(names([...inputs].reverse())).toEqual(names(inputs));
  });

  it('does not merge local and remote records implicitly', () => {
    const local = buildBranchTree([branch('feature/one')]);
    const remote = buildBranchTree([
      branch('origin/feature/one', {
        kind: 'remote',
        fullName: 'refs/remotes/origin/feature/one',
      }),
    ]);

    expect(local[0]).toMatchObject({ kind: 'folder', path: 'feature', branchCount: 1 });
    expect(remote[0]).toMatchObject({ kind: 'folder', path: 'origin', branchCount: 1 });
  });
});

describe('visibleBranchTree', () => {
  const tree = buildBranchTree([
    branch('main'),
    branch('rb/api/first'),
    branch('rb/api/second'),
    branch('rb/web/client'),
    branch('support/fix'),
  ]);

  it('flattens expanded nodes with depth and parent paths', () => {
    expect(
      visibleBranchTree(tree, new Set()).map((node) => ({
        kind: node.kind,
        path: node.path,
        depth: node.depth,
        parentPath: node.kind === 'branch' ? node.parentPath : undefined,
      })),
    ).toEqual([
      { kind: 'folder', path: 'rb', depth: 0, parentPath: undefined },
      { kind: 'folder', path: 'rb/api', depth: 1, parentPath: undefined },
      { kind: 'branch', path: 'rb/api/first', depth: 2, parentPath: 'rb/api' },
      { kind: 'branch', path: 'rb/api/second', depth: 2, parentPath: 'rb/api' },
      { kind: 'folder', path: 'rb/web', depth: 1, parentPath: undefined },
      { kind: 'branch', path: 'rb/web/client', depth: 2, parentPath: 'rb/web' },
      { kind: 'folder', path: 'support', depth: 0, parentPath: undefined },
      { kind: 'branch', path: 'support/fix', depth: 1, parentPath: 'support' },
      { kind: 'branch', path: 'main', depth: 0, parentPath: null },
    ]);
  });

  it('hides descendants of collapsed folders', () => {
    const visible = visibleBranchTree(tree, new Set(['rb/api']));

    expect(visible.find((node) => node.path === 'rb/api')).toMatchObject({
      kind: 'folder',
      collapsed: true,
    });
    expect(visible.some((node) => node.path === 'rb/api/first')).toBe(false);
    expect(visible.some((node) => node.path === 'rb/web/client')).toBe(true);
  });

  it('filters by normalized name path and retains all matching ancestors', () => {
    const visible = visibleBranchTree(tree, new Set(), 'API/SECOND');

    expect(visible.map((node) => node.path)).toEqual(['rb', 'rb/api', 'rb/api/second']);
  });

  it('filters by full ref name', () => {
    const visible = visibleBranchTree(tree, new Set(), 'refs/heads/support');

    expect(visible.map((node) => node.path)).toEqual(['support', 'support/fix']);
  });

  it('temporarily reveals matches below collapsed folders while filtering', () => {
    const visible = visibleBranchTree(tree, new Set(['rb', 'rb/api']), 'first');

    expect(visible.map((node) => node.path)).toEqual(['rb', 'rb/api', 'rb/api/first']);
    expect(
      visible.filter((node) => node.kind === 'folder').every((node) => !node.collapsed),
    ).toBe(true);
  });

  it('returns no nodes when no branch matches', () => {
    expect(visibleBranchTree(tree, new Set(), 'does-not-exist')).toEqual([]);
  });
});
