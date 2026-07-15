import { Injectable, InjectionToken, isDevMode } from '@angular/core';

export interface RepositoryStatusRequest {
  readonly repositoryPath: string;
}

export interface RepositoryBranchStatus {
  readonly oid: string | null;
  readonly head: string | null;
  readonly upstream: string | null;
  readonly ahead: number;
  readonly behind: number;
  readonly detached: boolean;
  readonly unborn: boolean;
}

export type StatusCode =
  | 'unmodified'
  | 'modified'
  | 'typeChanged'
  | 'added'
  | 'deleted'
  | 'renamed'
  | 'copied'
  | 'unmerged'
  | 'untracked'
  | 'ignored';

export interface StatusEntry {
  readonly kind: 'ordinary' | 'renamedOrCopied' | 'unmerged' | 'untracked' | 'ignored';
  readonly path: string;
  readonly originalPath: string | null;
  readonly indexStatus: StatusCode;
  readonly worktreeStatus: StatusCode;
  readonly submodule: string | null;
}

export interface RepositoryStatusResponse {
  readonly branch: RepositoryBranchStatus;
  readonly entries: readonly StatusEntry[];
}

export interface RepositoryHistoryRequest {
  readonly repositoryId: string;
  readonly cursor: string | null;
  readonly limit: number;
}

export interface RepositoryCommitSummary {
  readonly oid: string;
  readonly parents: readonly string[];
  readonly author: CommitAuthor;
  readonly summary: string;
  readonly refs: readonly string[];
}

export interface CommitAuthor {
  readonly name: string;
  readonly email: string;
  readonly authoredAt: string;
}

export interface RepositoryHistoryResponse {
  readonly commits: readonly RepositoryCommitSummary[];
  readonly nextCursor: string | null;
}

export interface RepositoryCommitDetailRequest {
  readonly repositoryId: string;
  readonly oid: string;
}

export type CommitFileStatus =
  | 'added'
  | 'modified'
  | 'deleted'
  | 'renamed'
  | 'copied'
  | 'typeChanged'
  | 'unmerged'
  | 'unknown';

export interface CommitChangedFile {
  readonly path: string;
  readonly oldPath: string | null;
  readonly status: CommitFileStatus;
  readonly additions: number | null;
  readonly deletions: number | null;
  readonly binary: boolean;
}

export interface RepositoryCommitDetailResponse {
  readonly oid: string;
  readonly parents: readonly string[];
  readonly author: CommitAuthor;
  readonly summary: string;
  readonly fullMessage: string;
  readonly refs: readonly string[];
  readonly files: readonly CommitChangedFile[];
}

export interface RepositoryFileDiffRequest {
  readonly repositoryId: string;
  readonly oid: string;
  readonly path: string;
  readonly oldPath: string | null;
}

export interface RepositoryFileDiffResponse {
  readonly oid: string;
  readonly path: string;
  readonly patch: string;
  readonly binary: boolean;
  readonly truncated: boolean;
}

export interface RepositoryNavigationRequest {
  readonly repositoryId: string;
}

export interface SwitchRepositoryBranchRequest {
  readonly repositoryId: string;
  readonly fullName: string;
}

export interface SwitchRepositoryBranchResponse {
  readonly fullName: string;
  readonly name: string;
  readonly head: string;
  readonly changed: boolean;
}

export interface RepositoryBranch {
  readonly kind: 'local' | 'remote';
  readonly fullName: string;
  readonly name: string;
  readonly oid: string;
  readonly current: boolean;
  readonly upstream: string | null;
  readonly ahead: number;
  readonly behind: number;
  readonly upstreamGone: boolean;
  readonly symbolicTarget: string | null;
}

export interface RepositoryWorktree {
  readonly path: string;
  readonly head: string | null;
  readonly branch: string | null;
  readonly detached: boolean;
  readonly bare: boolean;
  readonly locked: boolean;
  readonly lockReason: string | null;
  readonly prunable: boolean;
  readonly prunableReason: string | null;
}

export interface RepositoryStash {
  readonly oid: string;
  readonly selector: string;
  readonly message: string;
  readonly author: string;
  readonly authoredAt: string;
}

export interface RepositoryNavigationResponse {
  readonly branches: readonly RepositoryBranch[];
  readonly worktrees: readonly RepositoryWorktree[];
  readonly stashes: readonly RepositoryStash[];
}

export interface SelectRepositoryDirectoryRequest {
  readonly initialPath: string | null;
}

export interface SelectRepositoryDirectoryResponse {
  readonly path: string | null;
}

export type RepositoryProvider = 'local' | 'github' | 'other';
export type RepositoryTransport = 'local' | 'ssh' | 'https' | 'other';
export type RepositoryAvailability = 'unknown' | 'available' | 'missing' | 'inaccessible';
export type IntegrationHealthState = 'unknown' | 'healthy' | 'degraded' | 'unavailable';
export type IntegrationHealthIssue =
  | 'authentication'
  | 'authorization'
  | 'network'
  | 'notFound'
  | 'invalidConfiguration'
  | 'operationFailed';

export interface IntegrationHealth {
  readonly state: IntegrationHealthState;
  readonly issue: IntegrationHealthIssue | null;
  readonly checkedAt: number | null;
}

export interface HostedRepositoryIdentity {
  readonly host: string;
  readonly owner: string;
  readonly name: string;
}

export interface RememberedRepositoryResponse {
  readonly id: string;
  readonly canonicalPath: string;
  readonly displayName: string;
  readonly provider: RepositoryProvider;
  readonly transport: RepositoryTransport;
  readonly hostedIdentity: HostedRepositoryIdentity | null;
  readonly availability: RepositoryAvailability;
  readonly gitHealth: IntegrationHealth;
  readonly githubHealth: IntegrationHealth;
  readonly pinned: boolean;
  readonly openCount: number;
  readonly lastOpenedAt: number | null;
  readonly createdAt: number;
  readonly updatedAt: number;
}

export interface DesktopIpcContract {
  readonly repository_status: {
    readonly request: RepositoryStatusRequest;
    readonly response: RepositoryStatusResponse;
  };
  readonly repository_history: {
    readonly request: RepositoryHistoryRequest;
    readonly response: RepositoryHistoryResponse;
  };
  readonly repository_commit_detail: {
    readonly request: RepositoryCommitDetailRequest;
    readonly response: RepositoryCommitDetailResponse;
  };
  readonly repository_file_diff: {
    readonly request: RepositoryFileDiffRequest;
    readonly response: RepositoryFileDiffResponse;
  };
  readonly repository_navigation: {
    readonly request: RepositoryNavigationRequest;
    readonly response: RepositoryNavigationResponse;
  };
  readonly switch_repository_branch: {
    readonly request: SwitchRepositoryBranchRequest;
    readonly response: SwitchRepositoryBranchResponse;
  };
  readonly select_repository_directory: {
    readonly request: SelectRepositoryDirectoryRequest;
    readonly response: SelectRepositoryDirectoryResponse;
  };
  readonly list_remembered_repositories: {
    readonly request: Record<string, never>;
    readonly response: readonly RememberedRepositoryResponse[];
  };
  readonly remember_repository: {
    readonly request: RepositoryStatusRequest;
    readonly response: RememberedRepositoryResponse;
  };
  readonly set_repository_pinned: {
    readonly request: { readonly repositoryId: string; readonly pinned: boolean };
    readonly response: boolean;
  };
  readonly forget_repository: {
    readonly request: { readonly repositoryId: string };
    readonly response: boolean;
  };
}

type DesktopCommand = keyof DesktopIpcContract;

export interface DesktopIpcClient {
  invoke<C extends DesktopCommand>(
    command: C,
    request: DesktopIpcContract[C]['request'],
  ): Promise<DesktopIpcContract[C]['response']>;
}

interface TauriCore {
  invoke<TResponse>(command: string, arguments_?: object): Promise<TResponse>;
}

interface TauriGlobal {
  readonly __TAURI__?: {
    readonly core?: TauriCore;
  };
}

export const DESKTOP_IPC = new InjectionToken<DesktopIpcClient>('DESKTOP_IPC');

@Injectable({ providedIn: 'root' })
export class DesktopIpc implements DesktopIpcClient {
  private readonly core = (globalThis as TauriGlobal).__TAURI__?.core;

  invoke<C extends DesktopCommand>(
    command: C,
    request: DesktopIpcContract[C]['request'],
  ): Promise<DesktopIpcContract[C]['response']> {
    if (this.core !== undefined) {
      return this.core.invoke<DesktopIpcContract[C]['response']>(command, request);
    }

    if (isDevMode()) {
      return this.invokeMock(command, request);
    }

    return Promise.reject(
      new Error('The Tauri IPC bridge is unavailable in the production application.'),
    );
  }

  private invokeMock<C extends DesktopCommand>(
    command: C,
    request: DesktopIpcContract[C]['request'],
  ): Promise<DesktopIpcContract[C]['response']> {
    if (command === 'repository_status') {
      const response: RepositoryStatusResponse = {
        branch: {
          oid: 'a1b2c3d4e5f6',
          head: 'main',
          upstream: 'origin/main',
          ahead: 0,
          behind: 0,
          detached: false,
          unborn: false,
        },
        entries: [
          {
            kind: 'ordinary',
            path: 'src/app/app.ts',
            originalPath: null,
            indexStatus: 'unmodified',
            worktreeStatus: 'modified',
            submodule: null,
          },
        ],
      };

      return Promise.resolve(response as DesktopIpcContract[C]['response']);
    }

    if (command === 'select_repository_directory') {
      const response: SelectRepositoryDirectoryResponse = { path: null };
      return Promise.resolve(response as DesktopIpcContract[C]['response']);
    }

    if (command === 'repository_history') {
      const response: RepositoryHistoryResponse = { commits: [], nextCursor: null };
      return Promise.resolve(response as DesktopIpcContract[C]['response']);
    }

    if (command === 'repository_commit_detail') {
      return Promise.reject(
        new Error('Commit details are unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_file_diff') {
      return Promise.reject(
        new Error('File diffs are unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_navigation') {
      const response: RepositoryNavigationResponse = {
        branches: [],
        worktrees: [],
        stashes: [],
      };
      return Promise.resolve(response as DesktopIpcContract[C]['response']);
    }

    if (command === 'switch_repository_branch') {
      return Promise.reject(
        new Error('Switching branches is unavailable outside the desktop application.'),
      );
    }

    if (command === 'list_remembered_repositories') {
      return Promise.resolve([] as DesktopIpcContract[C]['response']);
    }

    if (command === 'remember_repository') {
      const repositoryPath = (request as RepositoryStatusRequest).repositoryPath;
      const displayName = repositoryPath.split(/[\\/]/).filter(Boolean).at(-1) ?? 'Repository';
      const now = Math.floor(Date.now() / 1000);
      const response: RememberedRepositoryResponse = {
        id: `browser-${displayName.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`,
        canonicalPath: repositoryPath,
        displayName,
        provider: 'local',
        transport: 'local',
        hostedIdentity: null,
        availability: 'available',
        gitHealth: { state: 'unknown', issue: null, checkedAt: null },
        githubHealth: { state: 'unknown', issue: null, checkedAt: null },
        pinned: false,
        openCount: 1,
        lastOpenedAt: now,
        createdAt: now,
        updatedAt: now,
      };
      return Promise.resolve(response as DesktopIpcContract[C]['response']);
    }

    if (command === 'set_repository_pinned' || command === 'forget_repository') {
      return Promise.resolve(true as DesktopIpcContract[C]['response']);
    }

    return Promise.reject(new Error(`Unsupported desktop command: ${command}`));
  }
}
