mod ai;
mod branch_actions;
mod branch_creation;
mod branch_operations;
mod clone;
mod commit_operations;
mod conflict_resolution;
mod file_diff;
mod file_history;
mod github;
mod history;
mod maintenance;
mod mutations;
mod navigation;
mod network_operations;
mod repository;
mod stash_actions;
mod stash_inspection;
mod status;
mod submodule;
mod working_tree;

pub use ai::{
    AiCliStatus, AiCliStatuses, AiCodeReviewDocument, AiCodeReviewList, AiCodeReviewSummary,
    AiGenerateCommitMessageRequest, AiGenerateCommitMessageResult, AiGenerateTaskReviewRequest,
    AiProvider, AiTaskReviewPreflightRequest, AiTaskReviewPreflightResult,
};
pub use branch_actions::{SwitchBranchRequest, SwitchBranchResult};
pub use branch_creation::{BranchCreationSource, CreateBranchRequest, CreateBranchResult};
pub use branch_operations::{
    MergeBranchRequest, MergeBranchResult, MergeBranchState, PullInactiveBranchRequest,
    PullInactiveBranchResult, WorktreeDirtyState,
};
pub use clone::{CloneRepositoryRequest, CloneRepositoryResult};
pub use commit_operations::{
    CommitOperationRequest, CommitOperationResult, CommitOperationState, ResetCommitRequest,
    ResetMode,
};
pub use conflict_resolution::{
    ConflictFileDetail, ConflictFileDetailRequest, ConflictFileSummary, ConflictListResult,
    ConflictResolution, ConflictStageIdentity, ConflictVersion, ResolveConflictRequest,
    ResolveConflictResult,
};
pub use file_diff::{FileDiff, FileDiffRequest};
pub use file_history::{FileBlame, FileBlameLine, FileBlameState, FileHistoryPage};
pub use github::{
    GitHubAccountState, GitHubAccountSummary, GitHubApiResult, GitHubAuthKind,
    GitHubDeviceFlowPoll, GitHubDeviceFlowStart, GitHubDeviceFlowState, GitHubPage,
    GitHubPatValidation, GitHubRateLimit, GitHubRepository, GitHubUser, IssueComment,
    PullRequestDetail, PullRequestFile, PullRequestMergeability, PullRequestState,
    PullRequestSummary, ReviewComment, ReviewCommentSide, ReviewThread,
};
pub use history::{
    ChangedFileStatus, ChangedFileSummary, CommitAuthor, CommitDetails, CommitHistoryPage,
    CommitListItem, CommitRelation, RefComparison, RefComparisonFileDiff,
};
pub use maintenance::{
    DeleteBranchRequest, DeleteBranchResult, FetchRepositoryResult, RemoveWorktreeRequest,
    RemoveWorktreeResult, WorktreeRemovalMode,
};
pub use mutations::{
    AmendCommitRequest, AmendCommitResult, AmendCommitState, ApplyIndexChangeRequest,
    ApplyIndexChangeResult, ChangeSelection, CreateCommitRequest, CreateCommitResult,
    DiscardWorkingTreeChangesRequest, DiscardWorkingTreeChangesResult,
    DiscardWorkingTreeHunkRequest, IndexAction, WorkingTreeEntrySelector,
};
pub use navigation::{
    RepositoryBranch, RepositoryBranchKind, RepositoryNavigation, RepositoryStash,
    RepositoryWorktree,
};
pub use network_operations::{
    PullOperationState, PullRequest, PullResult, PullStrategy, PushAnalysis, PushReadiness,
    PushRequest, PushResult, PushTarget, SetUpstreamRequest, SetUpstreamResult,
};
pub use repository::{
    HostedRepositoryIdentity, IntegrationHealth, IntegrationHealthIssue, IntegrationHealthState,
    RememberRepositoryInput, RememberedRepository, RepositoryAvailability, RepositoryGitIdentity,
    RepositoryGroupRelation, RepositoryHealthUpdate, RepositoryProvider, RepositoryRelationKind,
    RepositoryTransport, RepositoryWorktreeRole,
};
pub use stash_actions::{
    ApplyStashRequest, ApplyStashResult, AutoStashCreateState, AutoStashOptions, AutoStashOutcome,
    DropStashRequest, DropStashResult, PopStashRequest, PopStashResult, PushStashRequest,
    PushStashResult, RepositoryStatePrecondition, StashCleanupState, StashIdentity, StashPushState,
    StashRestoreState,
};
pub use stash_inspection::{
    StashChangedFile, StashDetails, StashFileDiff, StashFileDiffRequest, StashFileSource,
};
pub use status::{BranchStatus, RepositoryStatus, StatusCode, StatusEntry, StatusEntryKind};
pub use submodule::{
    RepositorySubmodule, RepositorySubmodules, SubmoduleCommitState, SubmoduleWorktreeState,
};
pub use working_tree::{WorkingTreeFileDiff, WorkingTreeFileDiffRequest};
