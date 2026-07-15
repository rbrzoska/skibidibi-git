mod branch_actions;
mod branch_creation;
mod file_diff;
mod history;
mod maintenance;
mod mutations;
mod navigation;
mod repository;
mod stash_actions;
mod status;
mod working_tree;

pub use branch_actions::{SwitchBranchRequest, SwitchBranchResult};
pub use branch_creation::{BranchCreationSource, CreateBranchRequest, CreateBranchResult};
pub use file_diff::{FileDiff, FileDiffRequest};
pub use history::{
    ChangedFileStatus, ChangedFileSummary, CommitAuthor, CommitDetails, CommitHistoryPage,
    CommitListItem,
};
pub use maintenance::{
    DeleteBranchRequest, DeleteBranchResult, FetchRepositoryResult, RemoveWorktreeRequest,
    RemoveWorktreeResult,
};
pub use mutations::{
    AmendCommitRequest, AmendCommitResult, AmendCommitState, ApplyIndexChangeRequest,
    ApplyIndexChangeResult, ChangeSelection, CreateCommitRequest, CreateCommitResult, IndexAction,
    WorkingTreeEntrySelector,
};
pub use navigation::{
    RepositoryBranch, RepositoryBranchKind, RepositoryNavigation, RepositoryStash,
    RepositoryWorktree,
};
pub use repository::{
    HostedRepositoryIdentity, IntegrationHealth, IntegrationHealthIssue, IntegrationHealthState,
    RememberRepositoryInput, RememberedRepository, RepositoryAvailability, RepositoryHealthUpdate,
    RepositoryProvider, RepositoryTransport,
};
pub use stash_actions::{
    ApplyStashRequest, ApplyStashResult, AutoStashCreateState, AutoStashOptions, AutoStashOutcome,
    DropStashRequest, DropStashResult, PopStashRequest, PopStashResult, PushStashRequest,
    PushStashResult, RepositoryStatePrecondition, StashCleanupState, StashIdentity, StashPushState,
    StashRestoreState,
};
pub use status::{BranchStatus, RepositoryStatus, StatusCode, StatusEntry, StatusEntryKind};
pub use working_tree::{WorkingTreeFileDiff, WorkingTreeFileDiffRequest};
