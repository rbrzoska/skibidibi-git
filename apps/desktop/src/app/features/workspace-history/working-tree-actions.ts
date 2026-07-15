import type { StatusEntry } from '../../core/ipc/desktop-ipc';
import type { WorkingTreeFile } from './working-tree-summary';

export interface WorkingTreeEntrySelector {
  readonly path: string;
  readonly oldPath: string | null;
  readonly entryKind: StatusEntry['kind'];
}

export type WorkingTreeSelection =
  | { readonly kind: 'all' }
  | { readonly kind: 'selected'; readonly entries: readonly WorkingTreeEntrySelector[] };

export type WorkingTreeMutationAction = 'stage' | 'unstage';

export interface WorkingTreeMutationPlan {
  readonly selection: WorkingTreeSelection;
  readonly actionableCount: number;
  readonly ambiguousPaths: readonly string[];
  readonly ambiguous: boolean;
}

export interface WorkingTreeActionCapabilities {
  readonly selectedCount: number;
  readonly stagedCount: number;
  readonly unstagedCount: number;
  readonly conflictedCount: number;
  readonly canStageSelected: boolean;
  readonly canUnstageSelected: boolean;
  readonly canStageAll: boolean;
  readonly canUnstageAll: boolean;
  readonly canCommit: boolean;
  readonly stageSelectedAmbiguous: boolean;
  readonly unstageSelectedAmbiguous: boolean;
}

/**
 * A path is not a sufficient identity: porcelain can report a staged deletion and an
 * untracked replacement for the same path. JSON encoding also avoids delimiter collisions.
 */
export function workingTreeFileKey(file: WorkingTreeFile): string {
  return JSON.stringify([
    file.entryKind,
    file.path,
    file.oldPath,
    file.primaryStatus,
  ]);
}

export function workingTreeEntrySelector(file: WorkingTreeFile): WorkingTreeEntrySelector {
  return {
    path: file.path,
    oldPath: file.oldPath,
    entryKind: file.entryKind,
  };
}

export function reconcileWorkingTreeSelection(
  selectedKeys: ReadonlySet<string>,
  files: readonly WorkingTreeFile[],
): ReadonlySet<string> {
  const availableKeys = new Set(
    files
      .filter(isSelectable)
      .map(workingTreeFileKey),
  );

  return new Set([...selectedKeys].filter((key) => availableKeys.has(key)));
}

export function planWorkingTreeMutation(
  action: WorkingTreeMutationAction,
  files: readonly WorkingTreeFile[],
  selection: 'all' | ReadonlySet<string>,
): WorkingTreeMutationPlan {
  const candidates = files.filter((file) => isActionable(file, action));
  if (selection === 'all') {
    return {
      selection: { kind: 'all' },
      actionableCount: candidates.length,
      ambiguousPaths: [],
      ambiguous: false,
    };
  }

  const reconciledSelection = reconcileWorkingTreeSelection(selection, files);
  const selectedCandidates = candidates.filter((file) =>
    reconciledSelection.has(workingTreeFileKey(file)),
  );
  const ambiguousPaths = findAmbiguousPaths(files, selectedCandidates, reconciledSelection);

  return {
    selection: {
      kind: 'selected',
      entries: selectedCandidates.map(workingTreeEntrySelector),
    },
    actionableCount: selectedCandidates.length,
    ambiguousPaths,
    ambiguous: ambiguousPaths.length > 0,
  };
}

export function workingTreeActionCapabilities(
  files: readonly WorkingTreeFile[],
  selectedKeys: ReadonlySet<string>,
): WorkingTreeActionCapabilities {
  const reconciledSelection = reconcileWorkingTreeSelection(selectedKeys, files);
  const stageSelected = planWorkingTreeMutation('stage', files, reconciledSelection);
  const unstageSelected = planWorkingTreeMutation('unstage', files, reconciledSelection);
  const stageAll = planWorkingTreeMutation('stage', files, 'all');
  const unstageAll = planWorkingTreeMutation('unstage', files, 'all');
  const selectableFiles = files.filter(isSelectable);
  const conflictedCount = files.filter((file) => file.primaryStatus === 'conflicted').length;

  return {
    selectedCount: reconciledSelection.size,
    stagedCount: selectableFiles.filter((file) => file.staged).length,
    unstagedCount: selectableFiles.filter((file) => file.unstaged).length,
    conflictedCount,
    canStageSelected:
      conflictedCount === 0 && stageSelected.actionableCount > 0 && !stageSelected.ambiguous,
    canUnstageSelected:
      conflictedCount === 0 && unstageSelected.actionableCount > 0 && !unstageSelected.ambiguous,
    canStageAll: conflictedCount === 0 && stageAll.actionableCount > 0,
    canUnstageAll: conflictedCount === 0 && unstageAll.actionableCount > 0,
    canCommit: selectableFiles.some((file) => file.staged) && conflictedCount === 0,
    stageSelectedAmbiguous: stageSelected.ambiguous,
    unstageSelectedAmbiguous: unstageSelected.ambiguous,
  };
}

function isSelectable(file: WorkingTreeFile): boolean {
  return file.primaryStatus !== 'conflicted' && (file.staged || file.unstaged);
}

function isActionable(file: WorkingTreeFile, action: WorkingTreeMutationAction): boolean {
  return isSelectable(file) && (action === 'stage' ? file.unstaged : file.staged);
}

function findAmbiguousPaths(
  files: readonly WorkingTreeFile[],
  selectedCandidates: readonly WorkingTreeFile[],
  selectedKeys: ReadonlySet<string>,
): readonly string[] {
  const selectedCandidatePaths = new Set(selectedCandidates.map((file) => file.path));
  const ambiguousPaths = new Set<string>();

  for (const path of selectedCandidatePaths) {
    const siblings = files.filter((file) => file.path === path && isSelectable(file));
    if (
      siblings.length > 1 &&
      siblings.some((file) => !selectedKeys.has(workingTreeFileKey(file)))
    ) {
      ambiguousPaths.add(path);
    }
  }

  return [...ambiguousPaths].sort((left, right) => left.localeCompare(right));
}
