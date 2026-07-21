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
  readonly indexFingerprint: string;
  readonly worktreeFingerprint: string;
}

export type IndexAction = 'stage' | 'unstage';

export interface WorkingTreeEntrySelector {
  readonly path: string;
  readonly oldPath: string | null;
  readonly entryKind: StatusEntry['kind'];
}

export type ChangeSelection =
  | { readonly scope: 'all' }
  | {
      readonly scope: 'selected';
      readonly entries: readonly WorkingTreeEntrySelector[];
    };

export interface ApplyIndexChangeOperation {
  readonly action: IndexAction;
  readonly selection: ChangeSelection;
  readonly expectedHead: string | null;
  readonly expectedHeadName: string | null;
  readonly expectedDetached: boolean;
  readonly expectedUnborn: boolean;
  readonly expectedIndexFingerprint: string;
  readonly expectedWorktreeFingerprint: string;
}

export interface ApplyIndexChangeRequest {
  readonly repositoryId: string;
  readonly operation: ApplyIndexChangeOperation;
}

export interface ApplyIndexChangeResponse {
  readonly changed: boolean;
  readonly status: RepositoryStatusResponse;
}

export interface CreateCommitOperation {
  readonly message: string;
  readonly expectedHead: string | null;
  readonly expectedHeadName: string | null;
  readonly expectedDetached: boolean;
  readonly expectedUnborn: boolean;
  readonly expectedIndexFingerprint: string;
  readonly expectedWorktreeFingerprint: string;
}

export interface CreateCommitRequest {
  readonly repositoryId: string;
  readonly operation: CreateCommitOperation;
}

export interface CreateCommitResponse {
  readonly oid: string;
  readonly status: RepositoryStatusResponse;
}

export interface AmendCommitOperation {
  readonly message: string | null;
  readonly confirmUpstreamRewrite: boolean;
  readonly expectedHead: string | null;
  readonly expectedHeadName: string | null;
  readonly expectedDetached: boolean;
  readonly expectedUnborn: boolean;
  readonly expectedIndexFingerprint: string;
  readonly expectedWorktreeFingerprint: string;
}

export interface AmendCommitRequest {
  readonly repositoryId: string;
  readonly operation: AmendCommitOperation;
}

export interface AmendCommitResponse {
  readonly previousOid: string;
  readonly oid: string | null;
  readonly status: RepositoryStatusResponse | null;
  readonly state: 'succeeded' | 'outcomeUnknown';
  readonly errorMessage: string | null;
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

export type StashFileSource = 'tracked' | 'untracked';

export interface StashChangedFile extends CommitChangedFile {
  readonly source: StashFileSource;
}

export interface RepositoryStashDetailRequest {
  readonly repositoryId: string;
  readonly oid: string;
}

export interface RepositoryStashDetailResponse {
  readonly oid: string;
  readonly files: readonly StashChangedFile[];
}

export interface RepositoryStashFileDiffRequest {
  readonly repositoryId: string;
  readonly oid: string;
  readonly source: StashFileSource;
  readonly path: string;
  readonly oldPath: string | null;
}

export interface RepositoryStashFileDiffResponse {
  readonly oid: string;
  readonly source: StashFileSource;
  readonly path: string;
  readonly patch: string;
  readonly binary: boolean;
  readonly truncated: boolean;
}

export interface WorkingTreeFileDiffRequest {
  readonly repositoryId: string;
  readonly path: string;
  readonly oldPath: string | null;
  readonly entryKind: StatusEntry['kind'];
}

export interface WorkingTreeFileDiffResponse {
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
  readonly operation: {
    readonly fullName: string;
    readonly expectedOid: string;
    readonly stashOnDirty: boolean;
    readonly stashMessage: string | null;
  };
}

export interface SwitchRepositoryBranchResponse {
  readonly fullName: string;
  readonly name: string;
  readonly head: string;
  readonly changed: boolean;
  readonly stashCreated: boolean;
  readonly operationSucceeded: boolean;
  readonly operationError: string | null;
  readonly autoStash: AutoStashOutcome;
}

export type BranchCreationSource =
  | { readonly kind: 'current'; readonly expectedOid: string }
  | { readonly kind: 'commit'; readonly oid: string }
  | {
      readonly kind: 'remoteTracking';
      readonly fullName: string;
      readonly expectedOid: string;
    };

export interface CreateRepositoryBranchRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly name: string | null;
    readonly source: BranchCreationSource;
  };
}

export interface CreateRepositoryBranchResponse {
  readonly fullName: string;
  readonly name: string;
  readonly head: string;
  readonly upstream: string | null;
}

export interface RepositoryStatePrecondition {
  readonly expectedHead: string | null;
  readonly expectedHeadName: string | null;
  readonly expectedDetached: boolean;
  readonly expectedUnborn: boolean;
  readonly expectedIndexFingerprint: string;
  readonly expectedWorktreeFingerprint: string;
}

export interface StashIdentity {
  readonly oid: string;
  readonly selector: string;
}

export type StashPushState = 'noChanges' | 'created' | 'failed' | 'partial';
export type AutoStashCreateState =
  | 'notRequested'
  | 'notNeeded'
  | 'created'
  | 'failed'
  | 'partial';
export type StashRestoreState =
  | 'notRequired'
  | 'applied'
  | 'conflicted'
  | 'failed'
  | 'skippedUnsafe';
export type StashCleanupState = 'notRequired' | 'dropped' | 'retained' | 'failed';

export interface AutoStashOutcome {
  readonly create: AutoStashCreateState;
  readonly stash: StashIdentity | null;
  readonly restore: StashRestoreState;
  readonly cleanup: StashCleanupState;
  readonly createError: string | null;
  readonly restoreError: string | null;
  readonly cleanupError: string | null;
}

export interface PushRepositoryStashRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly message: string;
    readonly includeUntracked: boolean;
    readonly precondition: RepositoryStatePrecondition;
  };
}

export interface PushRepositoryStashResponse {
  readonly state: StashPushState;
  readonly stash: StashIdentity | null;
  readonly status: RepositoryStatusResponse | null;
  readonly errorMessage: string | null;
  readonly mutationOid: string | null;
  readonly mutationMayHaveOccurred: boolean;
}

export interface ApplyRepositoryStashRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly stash: StashIdentity;
    readonly restoreIndex: boolean;
    readonly precondition: RepositoryStatePrecondition;
  };
}

export interface ApplyRepositoryStashResponse {
  readonly stash: StashIdentity;
  readonly restore: StashRestoreState;
  readonly cleanup: StashCleanupState;
  readonly status: RepositoryStatusResponse | null;
  readonly errorMessage: string | null;
  readonly mutationMayHaveOccurred: boolean;
}

export interface PopRepositoryStashRequest {
  readonly repositoryId: string;
  readonly operation: ApplyRepositoryStashRequest['operation'];
}

export interface PopRepositoryStashResponse {
  readonly stash: StashIdentity;
  readonly restore: StashRestoreState;
  readonly cleanup: StashCleanupState;
  readonly status: RepositoryStatusResponse | null;
  readonly restoreError: string | null;
  readonly cleanupError: string | null;
  readonly mutationMayHaveOccurred: boolean;
}

export interface DropRepositoryStashRequest {
  readonly repositoryId: string;
  readonly operation: { readonly stash: StashIdentity };
}

export interface DropRepositoryStashResponse {
  readonly stash: StashIdentity;
  readonly cleanup: StashCleanupState;
  readonly errorMessage: string | null;
  readonly mutationMayHaveOccurred: boolean;
}

export type PullStrategy = 'ffIfPossible' | 'ffOnly' | 'rebase';
export type PullOperationState = 'succeeded' | 'conflicted' | 'failed';

export interface PullRepositoryRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly strategy: PullStrategy;
    readonly autoStash: { readonly message: string } | null;
    readonly precondition: RepositoryStatePrecondition;
  };
}

export interface PullRepositoryResponse {
  readonly state: PullOperationState;
  readonly headBefore: string;
  readonly headAfter: string | null;
  readonly status: RepositoryStatusResponse | null;
  readonly autoStash: AutoStashOutcome;
  readonly errorMessage: string | null;
}

export interface MergeRepositoryBranchRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly sourceFullName: string;
    readonly expectedSourceOid: string;
    readonly targetFullName: string;
    readonly expectedTargetOid: string;
    readonly autoStash: { readonly message: string } | null;
  };
}

export interface MergeRepositoryBranchResponse {
  readonly state: PullOperationState;
  readonly headBefore: string;
  readonly headAfter: string | null;
  readonly status: RepositoryStatusResponse | null;
  readonly autoStash: AutoStashOutcome;
  readonly errorMessage: string | null;
  readonly mutationMayHaveOccurred: boolean;
}

export interface PullInactiveBranchRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly branchFullName: string;
    readonly expectedOid: string;
    readonly expectedUpstream: string;
  };
}

export interface PullInactiveBranchResponse {
  readonly branchFullName: string;
  readonly headBefore: string;
  readonly headAfter: string;
  readonly upstream: string;
  readonly changed: boolean;
}

export interface WorktreeDirtyStateResponse {
  readonly branchFullName: string | null;
  readonly worktreePath: string;
  readonly dirty: boolean;
  readonly changeCount: number;
  readonly errorMessage: string | null;
}

export interface WorktreeDirtyStatesResponse {
  readonly states: readonly WorktreeDirtyStateResponse[];
}

export type PushReadiness = 'noUpstream' | 'upToDate' | 'ready' | 'behind' | 'diverged';

export interface PushAnalysisResponse {
  readonly branch: string;
  readonly head: string;
  readonly upstream: string | null;
  readonly remote: string | null;
  readonly remoteRef: string | null;
  readonly ahead: number;
  readonly behind: number;
  readonly readiness: PushReadiness;
}

export type PushTarget =
  | { readonly kind: 'configured'; readonly expectedUpstream: string }
  | { readonly kind: 'setUpstream'; readonly remote: string; readonly remoteBranch: string };

export interface PushRepositoryRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly target: PushTarget;
    readonly precondition: RepositoryStatePrecondition;
  };
}

export interface PushRepositoryResponse {
  readonly pushed: boolean;
  readonly analysis: PushAnalysisResponse;
  readonly status: RepositoryStatusResponse;
}

export interface SetRepositoryUpstreamRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly remoteFullName: string;
    readonly expectedOid: string;
    readonly precondition: RepositoryStatePrecondition;
  };
}

export interface SetRepositoryUpstreamResponse {
  readonly upstream: string;
  readonly status: RepositoryStatusResponse;
}

export interface ConflictStageIdentity {
  readonly oid: string;
  readonly mode: string;
}

export interface ConflictFileSummary {
  readonly path: string;
  readonly base: ConflictStageIdentity | null;
  readonly ours: ConflictStageIdentity | null;
  readonly theirs: ConflictStageIdentity | null;
}

export interface ConflictListResponse {
  readonly files: readonly ConflictFileSummary[];
  readonly status: RepositoryStatusResponse;
}

export interface ConflictFileDetailRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly path: string;
    readonly expectedBase: ConflictStageIdentity | null;
    readonly expectedOurs: ConflictStageIdentity | null;
    readonly expectedTheirs: ConflictStageIdentity | null;
  };
}

export interface ConflictVersion {
  readonly identity: ConflictStageIdentity | null;
  readonly content: string | null;
  readonly binary: boolean;
}

export interface ConflictFileDetailResponse {
  readonly path: string;
  readonly base: ConflictVersion;
  readonly ours: ConflictVersion;
  readonly theirs: ConflictVersion;
  readonly workingContent: string | null;
  readonly workingBinary: boolean;
}

export type ConflictResolution =
  | { readonly kind: 'content'; readonly content: string }
  | { readonly kind: 'ours' }
  | { readonly kind: 'theirs' }
  | { readonly kind: 'delete' };

export interface ResolveConflictRequest {
  readonly repositoryId: string;
  readonly operation: {
    readonly path: string;
    readonly expectedBase: ConflictStageIdentity | null;
    readonly expectedOurs: ConflictStageIdentity | null;
    readonly expectedTheirs: ConflictStageIdentity | null;
    readonly resolution: ConflictResolution;
    readonly precondition: RepositoryStatePrecondition;
  };
}

export interface ResolveConflictResponse {
  readonly resolved: boolean;
  readonly status: RepositoryStatusResponse | null;
  readonly errorMessage: string | null;
  readonly mutationMayHaveOccurred: boolean;
}

export interface DeleteRepositoryBranchRequest {
  readonly repositoryId: string;
  readonly fullName: string;
  readonly expectedOid: string;
}

export interface DeleteRepositoryBranchResponse {
  readonly fullName: string;
  readonly deleted: boolean;
}

export interface RemoveRepositoryWorktreeRequest {
  readonly repositoryId: string;
  readonly path: string;
  readonly expectedHead: string | null;
  readonly branchFullName: string | null;
  readonly mode: WorktreeRemovalMode;
  readonly stashMessage: string | null;
}

export type WorktreeRemovalMode = 'safe' | 'force' | 'stashAndForce';

export interface RemoveRepositoryWorktreeResponse {
  readonly path: string;
  readonly branchFullName: string | null;
  readonly worktreeRemoved: boolean;
  readonly worktreeRemovalError: string | null;
  readonly branchDeleted: boolean;
  readonly branchDeletionError: string | null;
  readonly mode: WorktreeRemovalMode;
  readonly stash: StashIdentity | null;
}

export interface FetchRepositoryResponse {
  readonly fetchedAt: number;
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

export interface CloneRepositoryRequest {
  readonly sourceUrl: string;
  readonly destinationParent: string;
  readonly directoryName: string;
}

export interface RepositoryWorktreeStatisticsResponse {
  readonly path: string;
  readonly branch: string | null;
  readonly bytes: number;
  readonly lastCommitAt: string | null;
  readonly lastOpenedAt: number | null;
}

export interface RepositoryMaintenanceStatisticsResponse {
  readonly repositoryBytes: number;
  readonly gitBytes: number;
  readonly worktreeBytes: number;
  readonly scannedAt: number;
  readonly repositoryLastCommitAt: string | null;
  readonly worktrees: readonly RepositoryWorktreeStatisticsResponse[];
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
  readonly repositoryGroupId: string | null;
  readonly worktreeRole: 'main' | 'linked' | 'bare' | 'unknown';
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

/** A submodule declared by a remembered repository.  `currentOid` is null when
 * the module has not been initialised locally. */
export interface RepositorySubmodule {
  readonly name: string;
  readonly path: string;
  readonly url: string | null;
  readonly expectedOid: string | null;
  readonly currentOid: string | null;
  readonly present: boolean;
  readonly initialized: boolean;
  readonly commitState: 'atExpected' | 'different' | 'unavailable' | 'conflicted';
  readonly worktreeState:
    | 'clean'
    | 'modified'
    | 'untracked'
    | 'modifiedAndUntracked'
    | 'conflicted'
    | 'unavailable';
  readonly changeCount: number;
}

export interface RepositorySubmodulesResponse {
  readonly submodules: readonly RepositorySubmodule[];
}

export interface RepositoryGroupRelationResponse {
  readonly parentRepositoryGroupId: string;
  readonly childRepositoryGroupId: string;
  readonly kind: 'submodule';
  readonly relativePath: string;
}

export interface ApplicationZoomResponse {
  readonly scale: number;
}

export type AiCliProvider = 'codex' | 'claude' | 'cursor';

export interface AiCliStatus {
  readonly provider: AiCliProvider;
  readonly displayName: string;
  readonly available: boolean;
  readonly version: string | null;
  readonly detail: string | null;
}

export interface AiCliStatusResponse {
  readonly statuses: readonly AiCliStatus[];
}

export interface GenerateAiCommitMessageRequest {
  readonly repositoryId: string;
  readonly provider: AiCliProvider;
  readonly promptTemplate: string;
  readonly expectedHead: string | null;
  readonly indexFingerprint: string;
  readonly worktreeFingerprint: string;
}

export interface GenerateAiCommitMessageResponse {
  readonly message: string;
  readonly indexFingerprint: string;
  readonly worktreeFingerprint: string;
}

export interface DiagnosticsSettingsResponse {
  readonly dataDirectory: string;
  readonly maxLogKilobytes: number;
  readonly logFile: string;
}

export interface DiagnosticEntry {
  readonly timestampMs: number;
  readonly severity: 'info' | 'warning' | 'error';
  readonly subsystem: string;
  readonly eventCode: string;
  readonly message: string;
  readonly fields: Readonly<Record<string, string>>;
}

export interface DiagnosticLogResponse {
  readonly entries: readonly DiagnosticEntry[];
  readonly totalBytes: number;
  readonly truncated: boolean;
}

function unavailableAiCliStatus(provider: AiCliProvider, displayName: string): AiCliStatus {
  return {
    provider,
    displayName,
    available: false,
    version: null,
    detail: 'Unavailable outside the desktop application.',
  };
}

export interface GitHubAccountResponse {
  readonly id: string;
  readonly host: string;
  readonly login: string;
  readonly displayName: string | null;
  readonly avatarUrl: string | null;
  readonly authKind: 'personalAccessToken' | 'oAuthDevice' | 'gitHubCli';
  readonly scopes: readonly string[];
  readonly state: 'unknown' | 'connected' | 'authenticationRequired' | 'unavailable';
  readonly lastValidatedAt: number | null;
}

export interface GitHubDeviceFlowStartResponse {
  readonly flowId: string;
  readonly userCode: string;
  readonly verificationUri: string;
  readonly expiresAt: number;
  readonly intervalSeconds: number;
}

export interface GitHubDeviceFlowPollResponse {
  readonly state: 'pending' | 'authorized' | 'expired' | 'denied';
  readonly nextPollAt: number | null;
  readonly account: GitHubAccountResponse | null;
}

export interface GitHubRepositoryResponse {
  readonly id: string;
  readonly owner: string;
  readonly name: string;
  readonly fullName: string;
  readonly private: boolean;
  readonly updatedAt: string;
  readonly httpsCloneUrl: string;
  readonly sshCloneUrl: string;
}

export interface GitHubPullRequestSummaryResponse {
  readonly number: number;
  readonly title: string;
  readonly url: string;
  readonly state: 'open' | 'closed' | 'merged';
  readonly draft: boolean;
  readonly authorLogin: string;
  readonly headRefName: string;
  readonly baseRefName: string;
  readonly updatedAt: string;
  readonly authoredByViewer: boolean;
  readonly commentCount: number;
  readonly reviewRequestedFromViewer: boolean | null;
  readonly unresolvedThreadCount: number | null;
}

export interface GitHubPullRequestCommentResponse {
  readonly id: string;
  readonly authorLogin: string;
  readonly body: string;
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly url: string | null;
  readonly path: string | null;
  readonly line: number | null;
  readonly side: 'left' | 'right' | null;
}

export interface GitHubReviewThreadResponse {
  readonly id: string;
  readonly path: string;
  readonly line: number | null;
  readonly resolved: boolean;
  readonly outdated: boolean;
  readonly comments: readonly GitHubPullRequestCommentResponse[];
}

export interface GitHubPullRequestDetailResponse extends GitHubPullRequestSummaryResponse {
  readonly body: string;
  readonly additions: number;
  readonly deletions: number;
  readonly changedFiles: number;
  readonly mergeability: 'unknown' | 'mergeable' | 'conflicting';
  readonly comments: readonly GitHubPullRequestCommentResponse[];
  readonly reviewThreads: readonly GitHubReviewThreadResponse[];
  readonly conversationTruncated: boolean;
  readonly reviewThreadsTruncated: boolean;
}

export interface DesktopIpcContract {
  readonly set_application_zoom: {
    readonly request: { readonly scale: number };
    readonly response: ApplicationZoomResponse;
  };
  readonly ai_cli_status: {
    readonly request: Record<string, never>;
    readonly response: AiCliStatusResponse;
  };
  readonly ai_generate_commit_message: {
    readonly request: GenerateAiCommitMessageRequest;
    readonly response: GenerateAiCommitMessageResponse;
  };
  readonly diagnostics_settings: {
    readonly request: Record<string, never>;
    readonly response: DiagnosticsSettingsResponse;
  };
  readonly diagnostics_update_settings: {
    readonly request: { readonly dataDirectory: string; readonly maxLogKilobytes: number };
    readonly response: DiagnosticsSettingsResponse;
  };
  readonly diagnostics_read: {
    readonly request: Record<string, never>;
    readonly response: DiagnosticLogResponse;
  };
  readonly diagnostics_clear: {
    readonly request: Record<string, never>;
    readonly response: void;
  };
  readonly select_diagnostics_directory: {
    readonly request: { readonly initialPath: string | null };
    readonly response: SelectRepositoryDirectoryResponse;
  };
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
  readonly repository_stash_detail: {
    readonly request: RepositoryStashDetailRequest;
    readonly response: RepositoryStashDetailResponse;
  };
  readonly repository_stash_file_diff: {
    readonly request: RepositoryStashFileDiffRequest;
    readonly response: RepositoryStashFileDiffResponse;
  };
  readonly repository_working_tree_file_diff: {
    readonly request: WorkingTreeFileDiffRequest;
    readonly response: WorkingTreeFileDiffResponse;
  };
  readonly repository_apply_index_change: {
    readonly request: ApplyIndexChangeRequest;
    readonly response: ApplyIndexChangeResponse;
  };
  readonly repository_create_commit: {
    readonly request: CreateCommitRequest;
    readonly response: CreateCommitResponse;
  };
  readonly repository_amend_commit: {
    readonly request: AmendCommitRequest;
    readonly response: AmendCommitResponse;
  };
  readonly repository_navigation: {
    readonly request: RepositoryNavigationRequest;
    readonly response: RepositoryNavigationResponse;
  };
  readonly switch_repository_branch: {
    readonly request: SwitchRepositoryBranchRequest;
    readonly response: SwitchRepositoryBranchResponse;
  };
  readonly create_repository_branch: {
    readonly request: CreateRepositoryBranchRequest;
    readonly response: CreateRepositoryBranchResponse;
  };
  readonly repository_push_stash: {
    readonly request: PushRepositoryStashRequest;
    readonly response: PushRepositoryStashResponse;
  };
  readonly repository_apply_stash: {
    readonly request: ApplyRepositoryStashRequest;
    readonly response: ApplyRepositoryStashResponse;
  };
  readonly repository_pop_stash: {
    readonly request: PopRepositoryStashRequest;
    readonly response: PopRepositoryStashResponse;
  };
  readonly repository_drop_stash: {
    readonly request: DropRepositoryStashRequest;
    readonly response: DropRepositoryStashResponse;
  };
  readonly repository_push_analysis: {
    readonly request: RepositoryNavigationRequest;
    readonly response: PushAnalysisResponse;
  };
  readonly repository_pull: {
    readonly request: PullRepositoryRequest;
    readonly response: PullRepositoryResponse;
  };
  readonly repository_merge_branch: {
    readonly request: MergeRepositoryBranchRequest;
    readonly response: MergeRepositoryBranchResponse;
  };
  readonly repository_pull_inactive_branch: {
    readonly request: PullInactiveBranchRequest;
    readonly response: PullInactiveBranchResponse;
  };
  readonly repository_worktree_dirty_states: {
    readonly request: RepositoryNavigationRequest;
    readonly response: WorktreeDirtyStatesResponse;
  };
  readonly repository_push: {
    readonly request: PushRepositoryRequest;
    readonly response: PushRepositoryResponse;
  };
  readonly repository_set_upstream: {
    readonly request: SetRepositoryUpstreamRequest;
    readonly response: SetRepositoryUpstreamResponse;
  };
  readonly repository_conflicts: {
    readonly request: RepositoryNavigationRequest;
    readonly response: ConflictListResponse;
  };
  readonly repository_conflict_detail: {
    readonly request: ConflictFileDetailRequest;
    readonly response: ConflictFileDetailResponse;
  };
  readonly repository_resolve_conflict: {
    readonly request: ResolveConflictRequest;
    readonly response: ResolveConflictResponse;
  };
  readonly delete_repository_branch: {
    readonly request: DeleteRepositoryBranchRequest;
    readonly response: DeleteRepositoryBranchResponse;
  };
  readonly remove_repository_worktree: {
    readonly request: RemoveRepositoryWorktreeRequest;
    readonly response: RemoveRepositoryWorktreeResponse;
  };
  readonly repository_fetch: {
    readonly request: RepositoryNavigationRequest;
    readonly response: FetchRepositoryResponse;
  };
  readonly select_repository_directory: {
    readonly request: SelectRepositoryDirectoryRequest;
    readonly response: SelectRepositoryDirectoryResponse;
  };
  readonly select_clone_parent_directory: {
    readonly request: SelectRepositoryDirectoryRequest;
    readonly response: SelectRepositoryDirectoryResponse;
  };
  readonly clone_repository: {
    readonly request: CloneRepositoryRequest;
    readonly response: RememberedRepositoryResponse;
  };
  readonly repository_maintenance_stats: {
    readonly request: { readonly repositoryId: string };
    readonly response: RepositoryMaintenanceStatisticsResponse;
  };
  readonly list_remembered_repositories: {
    readonly request: Record<string, never>;
    readonly response: readonly RememberedRepositoryResponse[];
  };
  readonly list_repository_relations: {
    readonly request: Record<string, never>;
    readonly response: readonly RepositoryGroupRelationResponse[];
  };
  readonly repository_submodules: {
    readonly request: { readonly repositoryId: string };
    readonly response: RepositorySubmodulesResponse;
  };
  readonly open_submodule_repository: {
    readonly request: { readonly parentRepositoryId: string; readonly path: string };
    readonly response: RememberedRepositoryResponse;
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
  readonly github_list_accounts: {
    readonly request: Record<string, never>;
    readonly response: readonly GitHubAccountResponse[];
  };
  readonly github_start_device_flow: {
    readonly request: Record<string, never>;
    readonly response: GitHubDeviceFlowStartResponse;
  };
  readonly github_poll_device_flow: {
    readonly request: { readonly flowId: string };
    readonly response: GitHubDeviceFlowPollResponse;
  };
  readonly github_cancel_device_flow: {
    readonly request: { readonly flowId: string };
    readonly response: { readonly cancelled: boolean };
  };
  readonly github_open_device_verification: {
    readonly request: { readonly flowId: string };
    readonly response: void;
  };
  readonly github_connect_pat: {
    readonly request: { readonly token: string };
    readonly response: GitHubAccountResponse;
  };
  readonly github_connect_cli: {
    readonly request: Record<string, never>;
    readonly response: GitHubAccountResponse;
  };
  readonly github_disconnect_account: {
    readonly request: { readonly accountId: string };
    readonly response: { readonly disconnected: boolean };
  };
  readonly github_list_repositories: {
    readonly request: { readonly accountId: string; readonly cursor: string | null; readonly pageSize: number };
    readonly response: { readonly repositories: readonly GitHubRepositoryResponse[]; readonly nextCursor: string | null };
  };
  readonly github_list_pull_requests: {
    readonly request: {
      readonly accountId: string;
      readonly repositoryId: string;
      readonly scope: 'assignedToViewer' | 'authoredByViewer';
      readonly cursor: string | null;
      readonly pageSize: number;
    };
    readonly response: {
      readonly pullRequests: readonly GitHubPullRequestSummaryResponse[];
      readonly nextCursor: string | null;
    };
  };
  readonly github_pull_request_detail: {
    readonly request: {
      readonly accountId: string;
      readonly repositoryId: string;
      readonly number: number;
    };
    readonly response: GitHubPullRequestDetailResponse;
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
    if (command === 'set_application_zoom') {
      const scale = (request as { readonly scale: number }).scale;
      return Promise.resolve({ scale } as DesktopIpcContract[C]['response']);
    }

    if (command === 'ai_cli_status') {
      const response: AiCliStatusResponse = {
        statuses: [
          unavailableAiCliStatus('codex', 'Codex'),
          unavailableAiCliStatus('claude', 'Claude Code'),
          unavailableAiCliStatus('cursor', 'Cursor'),
        ],
      };
      return Promise.resolve(response as DesktopIpcContract[C]['response']);
    }

    if (command === 'ai_generate_commit_message') {
      return Promise.reject(
        new Error('AI commit-message generation is unavailable outside the desktop application.'),
      );
    }

    if (command === 'diagnostics_settings') {
      return Promise.resolve({
        dataDirectory: '~/.skibidibi-git',
        maxLogKilobytes: 256,
        logFile: '~/.skibidibi-git/diagnostics.jsonl',
      } as DesktopIpcContract[C]['response']);
    }

    if (command === 'diagnostics_update_settings') {
      const settings = request as { readonly dataDirectory: string; readonly maxLogKilobytes: number };
      return Promise.resolve({
        ...settings,
        logFile: `${settings.dataDirectory}/diagnostics.jsonl`,
      } as DesktopIpcContract[C]['response']);
    }

    if (command === 'diagnostics_read') {
      return Promise.resolve({ entries: [], totalBytes: 0, truncated: false } as DesktopIpcContract[C]['response']);
    }

    if (command === 'diagnostics_clear') {
      return Promise.resolve(undefined as DesktopIpcContract[C]['response']);
    }

    if (command === 'select_diagnostics_directory') {
      return Promise.resolve({ path: null } as DesktopIpcContract[C]['response']);
    }

    if (command === 'repository_status') {
      const response: RepositoryStatusResponse = {
        indexFingerprint: 'browser-development-index',
        worktreeFingerprint: 'browser-development-worktree',
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

    if (command === 'select_repository_directory' || command === 'select_clone_parent_directory') {
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

    if (command === 'repository_stash_detail') {
      return Promise.reject(
        new Error('Stash details are unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_stash_file_diff') {
      return Promise.reject(
        new Error('Stash file diffs are unavailable outside the desktop application.'),
      );
    }

    if (
      command === 'repository_push_analysis' ||
      command === 'repository_pull' ||
      command === 'repository_merge_branch' ||
      command === 'repository_pull_inactive_branch' ||
      command === 'repository_worktree_dirty_states' ||
      command === 'repository_push' ||
      command === 'repository_set_upstream'
    ) {
      return Promise.reject(
        new Error('Network operations are unavailable outside the desktop application.'),
      );
    }

    if (
      command === 'repository_conflicts' ||
      command === 'repository_conflict_detail' ||
      command === 'repository_resolve_conflict'
    ) {
      return Promise.reject(
        new Error('Conflict resolution is unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_working_tree_file_diff') {
      return Promise.reject(
        new Error('Working-tree file diffs are unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_apply_index_change') {
      return Promise.reject(
        new Error('Staging changes is unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_create_commit') {
      return Promise.reject(
        new Error('Creating commits is unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_amend_commit') {
      return Promise.reject(
        new Error('Amending commits is unavailable outside the desktop application.'),
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

    if (command === 'create_repository_branch') {
      return Promise.reject(
        new Error('Creating branches is unavailable outside the desktop application.'),
      );
    }

    if (
      command === 'repository_push_stash' ||
      command === 'repository_apply_stash' ||
      command === 'repository_pop_stash' ||
      command === 'repository_drop_stash'
    ) {
      return Promise.reject(
        new Error('Stash mutations are unavailable outside the desktop application.'),
      );
    }

    if (command === 'delete_repository_branch') {
      return Promise.reject(
        new Error('Deleting branches is unavailable outside the desktop application.'),
      );
    }

    if (command === 'remove_repository_worktree') {
      return Promise.reject(
        new Error('Removing worktrees is unavailable outside the desktop application.'),
      );
    }

    if (command === 'repository_fetch') {
      return Promise.reject(
        new Error('Fetching repositories is unavailable outside the desktop application.'),
      );
    }

    if (command === 'list_remembered_repositories' || command === 'list_repository_relations') {
      return Promise.resolve([] as DesktopIpcContract[C]['response']);
    }

    if (command === 'remember_repository') {
      const repositoryPath = (request as RepositoryStatusRequest).repositoryPath;
      const displayName = repositoryPath.split(/[\\/]/).filter(Boolean).at(-1) ?? 'Repository';
      const now = Math.floor(Date.now() / 1000);
      const response: RememberedRepositoryResponse = {
        id: `browser-${displayName.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`,
        repositoryGroupId: null,
        worktreeRole: 'unknown',
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

    if (command === 'github_list_accounts') {
      return Promise.resolve([] as DesktopIpcContract[C]['response']);
    }

    if (
      command === 'clone_repository' ||
      command === 'repository_submodules' ||
      command === 'open_submodule_repository' ||
      command === 'repository_maintenance_stats' ||
      command === 'github_start_device_flow' ||
      command === 'github_poll_device_flow' ||
      command === 'github_cancel_device_flow' ||
      command === 'github_open_device_verification' ||
      command === 'github_connect_pat' ||
      command === 'github_connect_cli' ||
      command === 'github_disconnect_account' ||
      command === 'github_list_repositories' ||
      command === 'github_list_pull_requests' ||
      command === 'github_pull_request_detail'
    ) {
      return Promise.reject(
        new Error('GitHub integration is unavailable outside the desktop application.'),
      );
    }

    return Promise.reject(new Error(`Unsupported desktop command: ${command}`));
  }
}
