import { describe, expect, it } from 'vitest';

import type { RepositoryStatusResponse, StatusCode, StatusEntry } from '../../core/ipc/desktop-ipc';
import { createWorkingTreeSummary } from './working-tree-summary';

function entry(
  path: string,
  indexStatus: StatusCode,
  worktreeStatus: StatusCode,
  overrides: Partial<StatusEntry> = {},
): StatusEntry {
  return {
    kind: 'ordinary',
    path,
    originalPath: null,
    indexStatus,
    worktreeStatus,
    submodule: null,
    ...overrides,
  };
}

function response(entries: readonly StatusEntry[]): RepositoryStatusResponse {
  return {
    indexFingerprint: 'index-v1:fixture',
    worktreeFingerprint: 'worktree-v1:fixture',
    branch: {
      oid: '0123456789abcdef0123456789abcdef01234567',
      head: 'main',
      upstream: 'origin/main',
      ahead: 0,
      behind: 0,
      detached: false,
      unborn: false,
    },
    entries,
  };
}

describe('createWorkingTreeSummary', () => {
  it('classifies mixed index and worktree changes and marks both sides independently', () => {
    const summary = createWorkingTreeSummary(response([
      entry('both.ts', 'modified', 'modified'),
      entry('staged.ts', 'added', 'unmodified'),
      entry('unstaged.ts', 'unmodified', 'modified'),
    ]));

    expect(summary.files).toEqual([
      { path: 'staged.ts', oldPath: null, entryKind: 'ordinary', primaryStatus: 'added', staged: true, unstaged: false },
      { path: 'both.ts', oldPath: null, entryKind: 'ordinary', primaryStatus: 'modified', staged: true, unstaged: true },
      { path: 'unstaged.ts', oldPath: null, entryKind: 'ordinary', primaryStatus: 'modified', staged: false, unstaged: true },
    ]);
  });

  it('treats porcelain untracked codes as unstaged only', () => {
    const summary = createWorkingTreeSummary(response([
      entry('new.ts', 'untracked', 'untracked', { kind: 'untracked' }),
    ]));

    expect(summary.files[0]).toEqual({
      path: 'new.ts',
      oldPath: null,
      entryKind: 'untracked',
      primaryStatus: 'added',
      staged: false,
      unstaged: true,
    });
  });

  it('preserves the old path and treats copy and rename records as renamed', () => {
    const summary = createWorkingTreeSummary(response([
      entry('new-name.ts', 'renamed', 'unmodified', {
        kind: 'renamedOrCopied',
        originalPath: 'old-name.ts',
      }),
      entry('copy.ts', 'copied', 'unmodified', {
        kind: 'renamedOrCopied',
        originalPath: 'source.ts',
      }),
    ]));

    expect(summary.files.map(({ path, oldPath, primaryStatus }) => ({ path, oldPath, primaryStatus })))
      .toEqual([
        { path: 'copy.ts', oldPath: 'source.ts', primaryStatus: 'renamed' },
        { path: 'new-name.ts', oldPath: 'old-name.ts', primaryStatus: 'renamed' },
      ]);
  });

  it('gives conflicts precedence over every other status', () => {
    const summary = createWorkingTreeSummary(response([
      entry('conflicted.ts', 'added', 'deleted', { kind: 'unmerged' }),
    ]));

    expect(summary.files[0]).toMatchObject({
      primaryStatus: 'conflicted',
      staged: true,
      unstaged: true,
    });
  });

  it('excludes ignored and unchanged entries', () => {
    const summary = createWorkingTreeSummary(response([
      entry('ignored.log', 'ignored', 'ignored', { kind: 'ignored' }),
      entry('clean.ts', 'unmodified', 'unmodified'),
    ]));

    expect(summary).toEqual({
      files: [],
      counts: { added: 0, modified: 0, renamed: 0, deleted: 0, conflicted: 0, total: 0 },
    });
  });

  it('counts the requested 2 added, 1 modified, 1 renamed and 4 deleted example once per file', () => {
    const summary = createWorkingTreeSummary(response([
      entry('added-z.ts', 'added', 'unmodified'),
      entry('added-a.ts', 'unmodified', 'untracked', { kind: 'untracked' }),
      entry('modified.ts', 'modified', 'modified'),
      entry('renamed.ts', 'renamed', 'modified', {
        kind: 'renamedOrCopied',
        originalPath: 'before.ts',
      }),
      entry('deleted-d.ts', 'deleted', 'unmodified'),
      entry('deleted-b.ts', 'unmodified', 'deleted'),
      entry('deleted-c.ts', 'deleted', 'modified'),
      entry('deleted-a.ts', 'added', 'deleted'),
    ]));

    expect(summary.counts).toEqual({
      added: 2,
      modified: 1,
      renamed: 1,
      deleted: 4,
      conflicted: 0,
      total: 8,
    });
    expect(summary.files.map((file) => `${file.primaryStatus}:${file.path}`)).toEqual([
      'renamed:renamed.ts',
      'deleted:deleted-a.ts',
      'deleted:deleted-b.ts',
      'deleted:deleted-c.ts',
      'deleted:deleted-d.ts',
      'added:added-a.ts',
      'added:added-z.ts',
      'modified:modified.ts',
    ]);
  });

  it('uses type changes as modified and keeps deterministic status-then-path ordering', () => {
    const input = [
      entry('z.ts', 'unmodified', 'typeChanged'),
      entry('a.ts', 'modified', 'unmodified'),
      entry('gone.ts', 'deleted', 'unmodified'),
    ];

    expect(createWorkingTreeSummary(response(input)).files).toEqual(
      createWorkingTreeSummary(response([...input].reverse())).files,
    );
  });

  it('keeps a staged deletion and an untracked replacement at the same path distinct', () => {
    const summary = createWorkingTreeSummary(response([
      entry('same.txt', 'deleted', 'unmodified'),
      entry('same.txt', 'untracked', 'untracked', { kind: 'untracked' }),
    ]));

    expect(summary.files).toEqual([
      {
        path: 'same.txt',
        oldPath: null,
        entryKind: 'ordinary',
        primaryStatus: 'deleted',
        staged: true,
        unstaged: false,
      },
      {
        path: 'same.txt',
        oldPath: null,
        entryKind: 'untracked',
        primaryStatus: 'added',
        staged: false,
        unstaged: true,
      },
    ]);
    expect(summary.counts).toMatchObject({ added: 1, deleted: 1, total: 2 });
  });
});
