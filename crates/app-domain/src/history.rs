use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitAuthor {
    pub name: String,
    pub email: String,
    /// ISO 8601 timestamp emitted by Git (`%aI`).
    pub authored_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitListItem {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: CommitAuthor,
    pub summary: String,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitHistoryPage {
    pub commits: Vec<CommitListItem>,
    /// Opaque continuation token. It is the last OID returned by this page.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangedFileStatus {
    Added,
    Copied,
    Deleted,
    Modified,
    Renamed,
    TypeChanged,
    Unmerged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFileSummary {
    pub status: ChangedFileStatus,
    pub path: String,
    pub old_path: Option<String>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetails {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: CommitAuthor,
    pub summary: String,
    pub full_message: String,
    pub refs: Vec<String>,
    pub files: Vec<ChangedFileSummary>,
}
