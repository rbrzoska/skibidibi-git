use serde::{Deserialize, Serialize};

use crate::{AutoStashOptions, AutoStashOutcome, RepositoryStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeBranchRequest {
    pub source_full_name: String,
    pub expected_source_oid: String,
    pub target_full_name: String,
    pub expected_target_oid: String,
    pub auto_stash: Option<AutoStashOptions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeBranchState {
    Succeeded,
    Conflicted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeBranchResult {
    pub state: MergeBranchState,
    pub head_before: String,
    pub head_after: Option<String>,
    pub status: Option<RepositoryStatus>,
    pub auto_stash: AutoStashOutcome,
    pub error_message: Option<String>,
    pub mutation_may_have_occurred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullInactiveBranchRequest {
    pub branch_full_name: String,
    pub expected_oid: String,
    pub expected_upstream: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullInactiveBranchResult {
    pub branch_full_name: String,
    pub head_before: String,
    pub head_after: String,
    pub upstream: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDirtyState {
    pub branch_full_name: Option<String>,
    pub worktree_path: String,
    pub dirty: bool,
    pub change_count: usize,
    pub error_message: Option<String>,
}
