use serde::{Deserialize, Serialize};

use crate::RepositoryStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashIdentity {
    pub oid: String,
    /// Snapshot selector for display only. Runtime operations rebind the immutable OID.
    pub selector: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStatePrecondition {
    pub expected_head: Option<String>,
    pub expected_head_name: Option<String>,
    pub expected_detached: bool,
    pub expected_unborn: bool,
    pub expected_index_fingerprint: String,
    pub expected_worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushStashRequest {
    pub message: String,
    pub include_untracked: bool,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyStashRequest {
    pub stash: StashIdentity,
    pub restore_index: bool,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PopStashRequest {
    pub stash: StashIdentity,
    pub restore_index: bool,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropStashRequest {
    pub stash: StashIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StashPushState {
    NoChanges,
    Created,
    Failed,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StashRestoreState {
    NotRequired,
    Applied,
    Conflicted,
    Failed,
    SkippedUnsafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StashCleanupState {
    NotRequired,
    Dropped,
    Retained,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushStashResult {
    pub state: StashPushState,
    pub stash: Option<StashIdentity>,
    /// Absent only when the post-mutation status query itself failed.
    pub status: Option<RepositoryStatus>,
    pub error_message: Option<String>,
    /// Exact OID returned by `stash create`, or conservatively attributed after porcelain push.
    pub mutation_oid: Option<String>,
    /// True once a command capable of changing the stash reflog or working tree was attempted.
    pub mutation_may_have_occurred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyStashResult {
    pub stash: StashIdentity,
    pub restore: StashRestoreState,
    pub cleanup: StashCleanupState,
    pub status: Option<RepositoryStatus>,
    pub error_message: Option<String>,
    /// True once stash application was attempted, including conflicts and timeouts.
    pub mutation_may_have_occurred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PopStashResult {
    pub stash: StashIdentity,
    pub restore: StashRestoreState,
    pub cleanup: StashCleanupState,
    pub status: Option<RepositoryStatus>,
    pub restore_error: Option<String>,
    pub cleanup_error: Option<String>,
    /// Includes either apply or cleanup mutation attempts.
    pub mutation_may_have_occurred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropStashResult {
    pub stash: StashIdentity,
    pub cleanup: StashCleanupState,
    pub error_message: Option<String>,
    /// True once Git received the drop command, even when reconciliation later failed.
    pub mutation_may_have_occurred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoStashOptions {
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoStashCreateState {
    NotRequested,
    NotNeeded,
    Created,
    Failed,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoStashOutcome {
    pub create: AutoStashCreateState,
    pub stash: Option<StashIdentity>,
    pub restore: StashRestoreState,
    pub cleanup: StashCleanupState,
    pub create_error: Option<String>,
    pub restore_error: Option<String>,
    pub cleanup_error: Option<String>,
}

impl AutoStashOutcome {
    pub fn not_requested() -> Self {
        Self {
            create: AutoStashCreateState::NotRequested,
            stash: None,
            restore: StashRestoreState::NotRequired,
            cleanup: StashCleanupState::NotRequired,
            create_error: None,
            restore_error: None,
            cleanup_error: None,
        }
    }
}
