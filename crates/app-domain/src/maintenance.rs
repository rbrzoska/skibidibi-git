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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorktreeResult {
    pub path: String,
    pub branch_full_name: Option<String>,
    pub worktree_removed: bool,
    pub branch_deleted: bool,
    pub branch_deletion_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchRepositoryResult {
    pub fetched_at: i64,
}
