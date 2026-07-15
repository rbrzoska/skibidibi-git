import type {
  RepositoryStatusResponse,
  StatusCode,
  StatusEntry,
} from '../../core/ipc/desktop-ipc';

export type WorkingTreePrimaryStatus =
  | 'added'
  | 'modified'
  | 'renamed'
  | 'deleted'
  | 'conflicted';

export interface WorkingTreeFile {
  readonly path: string;
  readonly oldPath: string | null;
  readonly entryKind: StatusEntry['kind'];
  readonly primaryStatus: WorkingTreePrimaryStatus;
  readonly staged: boolean;
  readonly unstaged: boolean;
}

export interface WorkingTreeCounts {
  readonly added: number;
  readonly modified: number;
  readonly renamed: number;
  readonly deleted: number;
  readonly conflicted: number;
  readonly total: number;
}

export interface WorkingTreeSummary {
  readonly files: readonly WorkingTreeFile[];
  readonly counts: WorkingTreeCounts;
}

const STATUS_ORDER: Readonly<Record<WorkingTreePrimaryStatus, number>> = {
  conflicted: 0,
  renamed: 1,
  deleted: 2,
  added: 3,
  modified: 4,
};

const MODIFIED_CODES = new Set<StatusCode>(['modified', 'typeChanged']);
const RENAMED_CODES = new Set<StatusCode>(['renamed', 'copied']);

function hasStatus(entry: StatusEntry, statuses: ReadonlySet<StatusCode>): boolean {
  return statuses.has(entry.indexStatus) || statuses.has(entry.worktreeStatus);
}

function primaryStatus(entry: StatusEntry): WorkingTreePrimaryStatus | null {
  if (
    entry.kind === 'ignored' ||
    entry.indexStatus === 'ignored' ||
    entry.worktreeStatus === 'ignored'
  ) {
    return null;
  }

  if (
    entry.kind === 'unmerged' ||
    entry.indexStatus === 'unmerged' ||
    entry.worktreeStatus === 'unmerged'
  ) {
    return 'conflicted';
  }
  if (entry.kind === 'renamedOrCopied' || hasStatus(entry, RENAMED_CODES)) {
    return 'renamed';
  }
  if (entry.indexStatus === 'deleted' || entry.worktreeStatus === 'deleted') {
    return 'deleted';
  }
  if (
    entry.kind === 'untracked' ||
    entry.indexStatus === 'added' ||
    entry.worktreeStatus === 'added' ||
    entry.indexStatus === 'untracked' ||
    entry.worktreeStatus === 'untracked'
  ) {
    return 'added';
  }
  if (hasStatus(entry, MODIFIED_CODES)) {
    return 'modified';
  }
  return null;
}

function isStaged(entry: StatusEntry): boolean {
  return (
    entry.kind !== 'untracked' &&
    entry.indexStatus !== 'unmodified' &&
    entry.indexStatus !== 'ignored'
  );
}

function isUnstaged(entry: StatusEntry): boolean {
  return (
    entry.kind === 'untracked' ||
    (entry.worktreeStatus !== 'unmodified' && entry.worktreeStatus !== 'ignored')
  );
}

export function createWorkingTreeSummary(
  status: RepositoryStatusResponse,
): WorkingTreeSummary {
  const files = status.entries
    .map((entry): WorkingTreeFile | null => {
      const status = primaryStatus(entry);
      if (status === null) {
        return null;
      }
      return {
        path: entry.path,
        oldPath: entry.originalPath,
        entryKind: entry.kind,
        primaryStatus: status,
        staged: isStaged(entry),
        unstaged: isUnstaged(entry),
      };
    })
    .filter((file): file is WorkingTreeFile => file !== null)
    .sort(
      (left, right) =>
        STATUS_ORDER[left.primaryStatus] - STATUS_ORDER[right.primaryStatus] ||
        left.path.localeCompare(right.path),
    );

  const counts = files.reduce<WorkingTreeCounts>(
    (result, file) => ({
      ...result,
      [file.primaryStatus]: result[file.primaryStatus] + 1,
      total: result.total + 1,
    }),
    { added: 0, modified: 0, renamed: 0, deleted: 0, conflicted: 0, total: 0 },
  );

  return { files, counts };
}
