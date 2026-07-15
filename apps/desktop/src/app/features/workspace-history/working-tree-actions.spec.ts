import { describe, expect, it } from 'vitest';

import type { WorkingTreeFile } from './working-tree-summary';
import {
  planWorkingTreeMutation,
  reconcileWorkingTreeSelection,
  workingTreeActionCapabilities,
  workingTreeEntrySelector,
  workingTreeFileKey,
} from './working-tree-actions';

function file(overrides: Partial<WorkingTreeFile> = {}): WorkingTreeFile {
  return {
    path: 'src/app.ts',
    oldPath: null,
    entryKind: 'ordinary',
    primaryStatus: 'modified',
    staged: false,
    unstaged: true,
    ...overrides,
  };
}

describe('working-tree actions', () => {
  it('builds stable distinct keys from the complete working-tree identity', () => {
    const deleted = file({ path: 'same.txt', primaryStatus: 'deleted', staged: true, unstaged: false });
    const replacement = file({
      path: 'same.txt',
      entryKind: 'untracked',
      primaryStatus: 'added',
    });
    const renamed = file({
      path: 'after:name.ts',
      oldPath: 'before:name.ts',
      entryKind: 'renamedOrCopied',
      primaryStatus: 'renamed',
      staged: true,
      unstaged: false,
    });

    expect(workingTreeFileKey(deleted)).not.toBe(workingTreeFileKey(replacement));
    expect(workingTreeFileKey(renamed)).toBe(
      workingTreeFileKey({ ...renamed }),
    );
    expect(new Set([workingTreeFileKey(deleted), workingTreeFileKey(replacement)])).toHaveLength(2);
  });

  it('maps a file to the frozen entry selector contract', () => {
    expect(workingTreeEntrySelector(file({
      path: 'new.ts',
      oldPath: 'old.ts',
      entryKind: 'renamedOrCopied',
    }))).toEqual({
      path: 'new.ts',
      oldPath: 'old.ts',
      entryKind: 'renamedOrCopied',
    });
  });

  it('reconciles removed records and excludes conflicts from selection', () => {
    const remaining = file({ path: 'remaining.ts' });
    const removed = file({ path: 'removed.ts' });
    const conflict = file({
      path: 'conflict.ts',
      primaryStatus: 'conflicted',
      staged: true,
      unstaged: true,
    });

    expect(reconcileWorkingTreeSelection(
      new Set([workingTreeFileKey(remaining), workingTreeFileKey(removed), workingTreeFileKey(conflict)]),
      [remaining, conflict],
    )).toEqual(new Set([workingTreeFileKey(remaining)]));
  });

  it('plans selected stage and unstage operations from the matching side only', () => {
    const unstaged = file({ path: 'unstaged.ts' });
    const staged = file({ path: 'staged.ts', staged: true, unstaged: false });
    const both = file({ path: 'both.ts', staged: true, unstaged: true });
    const selected = new Set([unstaged, staged, both].map(workingTreeFileKey));

    expect(planWorkingTreeMutation('stage', [unstaged, staged, both], selected)).toEqual({
      selection: {
        kind: 'selected',
        entries: [workingTreeEntrySelector(unstaged), workingTreeEntrySelector(both)],
      },
      actionableCount: 2,
      ambiguousPaths: [],
      ambiguous: false,
    });
    expect(planWorkingTreeMutation('unstage', [unstaged, staged, both], selected)).toEqual({
      selection: {
        kind: 'selected',
        entries: [workingTreeEntrySelector(staged), workingTreeEntrySelector(both)],
      },
      actionableCount: 2,
      ambiguousPaths: [],
      ambiguous: false,
    });
  });

  it('uses an explicit all selection while still reporting its actionable count', () => {
    const staged = file({ path: 'staged.ts', staged: true, unstaged: false });
    const unstaged = file({ path: 'unstaged.ts' });
    const conflict = file({
      path: 'conflict.ts',
      primaryStatus: 'conflicted',
      staged: true,
      unstaged: true,
    });

    expect(planWorkingTreeMutation('stage', [staged, unstaged, conflict], 'all')).toEqual({
      selection: { kind: 'all' },
      actionableCount: 1,
      ambiguousPaths: [],
      ambiguous: false,
    });
  });

  it('marks a partial same-path sibling group as ambiguous for selected mutation', () => {
    const stagedDeletion = file({
      path: 'same.txt',
      primaryStatus: 'deleted',
      staged: true,
      unstaged: false,
    });
    const untrackedReplacement = file({
      path: 'same.txt',
      entryKind: 'untracked',
      primaryStatus: 'added',
      staged: false,
      unstaged: true,
    });
    const onlyReplacement = new Set([workingTreeFileKey(untrackedReplacement)]);

    expect(planWorkingTreeMutation(
      'stage',
      [stagedDeletion, untrackedReplacement],
      onlyReplacement,
    )).toMatchObject({
      actionableCount: 1,
      ambiguousPaths: ['same.txt'],
      ambiguous: true,
    });
    expect(workingTreeActionCapabilities(
      [stagedDeletion, untrackedReplacement],
      onlyReplacement,
    )).toMatchObject({
      canStageSelected: false,
      stageSelectedAmbiguous: true,
      canStageAll: true,
    });
  });

  it('allows a same-path sibling mutation when the complete group is selected', () => {
    const stagedDeletion = file({
      path: 'same.txt',
      primaryStatus: 'deleted',
      staged: true,
      unstaged: false,
    });
    const untrackedReplacement = file({
      path: 'same.txt',
      entryKind: 'untracked',
      primaryStatus: 'added',
    });
    const allSiblings = new Set([stagedDeletion, untrackedReplacement].map(workingTreeFileKey));

    expect(planWorkingTreeMutation(
      'stage',
      [stagedDeletion, untrackedReplacement],
      allSiblings,
    )).toMatchObject({
      actionableCount: 1,
      ambiguousPaths: [],
      ambiguous: false,
    });
  });

  it('derives stage, unstage and commit capabilities and excludes conflicts', () => {
    const staged = file({ path: 'staged.ts', staged: true, unstaged: false });
    const unstaged = file({ path: 'unstaged.ts' });
    const both = file({ path: 'both.ts', staged: true, unstaged: true });
    const conflict = file({
      path: 'conflict.ts',
      primaryStatus: 'conflicted',
      staged: true,
      unstaged: true,
    });
    const selected = new Set([staged, unstaged, both, conflict].map(workingTreeFileKey));

    expect(workingTreeActionCapabilities([staged, unstaged, both, conflict], selected)).toEqual({
      selectedCount: 3,
      stagedCount: 2,
      unstagedCount: 2,
      conflictedCount: 1,
      canStageSelected: false,
      canUnstageSelected: false,
      canStageAll: false,
      canUnstageAll: false,
      canCommit: false,
      stageSelectedAmbiguous: false,
      unstageSelectedAmbiguous: false,
    });

    expect(workingTreeActionCapabilities([staged, unstaged, both], selected).canCommit).toBe(true);
    expect(workingTreeActionCapabilities([unstaged], new Set()).canCommit).toBe(false);
  });
});
