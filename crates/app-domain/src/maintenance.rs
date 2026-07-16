use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBranchRequest {
    pub full_name: String,
    pub expected_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBranchResult {
    pub full_name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorktreeRequest {
    pub path: String,
    pub expected_head: Option<String>,
    pub branch_full_name: Option<String>,
    pub mode: WorktreeRemovalMode,
    pub stash_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorktreeRemovalMode {
    Safe,
    Force,
    StashAndForce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorktreeResult {
    pub path: String,
    pub branch_full_name: Option<String>,
    pub worktree_removed: bool,
    pub worktree_removal_error: Option<String>,
    pub branch_deleted: bool,
    pub branch_deletion_error: Option<String>,
    pub mode: WorktreeRemovalMode,
    pub stash: Option<crate::StashIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchRepositoryResult {
    pub fetched_at: i64,
}
