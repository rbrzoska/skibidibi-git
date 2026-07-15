use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStatus {
    pub branch: BranchStatus,
    pub entries: Vec<StatusEntry>,
    #[serde(default)]
    pub index_fingerprint: String,
    #[serde(default)]
    pub worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BranchStatus {
    pub oid: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub detached: bool,
    pub unborn: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub kind: StatusEntryKind,
    pub path: String,
    pub original_path: Option<String>,
    pub index_status: StatusCode,
    pub worktree_status: StatusCode,
    pub submodule: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusEntryKind {
    Ordinary,
    RenamedOrCopied,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusCode {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}
