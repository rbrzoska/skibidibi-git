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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<CommitRelation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommitRelation {
    Task,
    Merge,
    Base,
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

/// A stable, read-only comparison between two exact branch snapshots.
///
/// Ref names are retained for display while every Git query is executed against the immutable
/// object ids. The native boundary verifies both refs before and after building this response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefComparison {
    pub source_full_name: String,
    pub source_oid: String,
    pub target_full_name: String,
    pub target_oid: String,
    pub merge_base_oid: String,
    pub ahead: u64,
    pub behind: u64,
    pub commits: Vec<CommitListItem>,
    pub commits_truncated: bool,
    pub files: Vec<ChangedFileSummary>,
    pub files_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefComparisonFileDiff {
    pub source_full_name: String,
    pub source_oid: String,
    pub target_full_name: String,
    pub target_oid: String,
    pub path: String,
    pub old_path: Option<String>,
    pub patch: String,
    pub binary: bool,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::RefComparisonFileDiff;

    #[test]
    fn ref_comparison_file_diff_serializes_rename_identity_in_camel_case() {
        let response = RefComparisonFileDiff {
            source_full_name: "refs/heads/feature".to_owned(),
            source_oid: "a".repeat(40),
            target_full_name: "refs/heads/main".to_owned(),
            target_oid: "b".repeat(40),
            path: "new-name.rs".to_owned(),
            old_path: Some("old-name.rs".to_owned()),
            patch: String::new(),
            binary: false,
            truncated: false,
        };

        let value = serde_json::to_value(response).expect("serialize file diff");

        assert_eq!(value["oldPath"], "old-name.rs");
        assert!(value.get("old_path").is_none());
    }
}
