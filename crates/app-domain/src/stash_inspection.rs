use serde::{Deserialize, Serialize};

use crate::ChangedFileStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StashFileSource {
    Tracked,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashChangedFile {
    pub source: StashFileSource,
    pub status: ChangedFileStatus,
    pub path: String,
    pub old_path: Option<String>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashDetails {
    pub oid: String,
    pub files: Vec<StashChangedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashFileDiffRequest {
    pub oid: String,
    pub source: StashFileSource,
    pub path: String,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashFileDiff {
    pub oid: String,
    pub source: StashFileSource,
    pub path: String,
    pub patch: String,
    pub binary: bool,
    /// Successful results are complete. Output-limit failures are returned as errors.
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_frontend_friendly_source_and_paths() {
        let request = StashFileDiffRequest {
            oid: "a".repeat(40),
            source: StashFileSource::Tracked,
            path: "new name.txt".to_owned(),
            old_path: Some("old name.txt".to_owned()),
        };

        let value = serde_json::to_value(request).expect("serialize request");
        assert_eq!(value["source"], "tracked");
        assert_eq!(value["oldPath"], "old name.txt");
    }
}
