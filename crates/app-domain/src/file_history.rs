use serde::{Deserialize, Serialize};

use crate::CommitListItem;

/// One immutable, path-scoped history page. `start_oid` is the commit snapshot used for every
/// request, never a movable ref such as `HEAD` or a branch name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryPage {
    pub start_oid: String,
    pub path: String,
    pub commits: Vec<CommitListItem>,
    /// Opaque continuation token bound to `start_oid` and `path`.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileBlameState {
    Available,
    Binary,
    Oversized,
}

/// A single source line together with the immutable commit that last touched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileBlameLine {
    pub line_number: u64,
    pub oid: String,
    pub original_line_number: u64,
    pub final_line_number: u64,
    pub author_name: String,
    pub author_email: String,
    /// Git's strict ISO-8601 author timestamp, derived from porcelain `author-time` and
    /// `author-tz` rather than from the caller's locale.
    pub authored_at: String,
    pub summary: String,
    pub content: String,
}

/// Blame data for one exact commit/path snapshot.
///
/// Binary and oversized blobs return an explicit non-available state rather than attempting to
/// send an unsafe/unbounded payload to the renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileBlame {
    pub oid: String,
    pub path: String,
    pub state: FileBlameState,
    pub lines: Vec<FileBlameLine>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::{FileBlame, FileBlameState};

    #[test]
    fn blame_state_uses_camel_case_values() {
        let response = FileBlame {
            oid: "a".repeat(40),
            path: "src/file.rs".to_owned(),
            state: FileBlameState::Oversized,
            lines: Vec::new(),
            truncated: false,
        };

        let value = serde_json::to_value(response).expect("serialize blame response");
        assert_eq!(value["state"], "oversized");
        assert!(value.get("truncated").is_some());
    }
}
