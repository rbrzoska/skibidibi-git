use serde::{Deserialize, Serialize};

use crate::StatusEntryKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTreeFileDiffRequest {
    pub path: String,
    pub old_path: Option<String>,
    /// Disambiguates porcelain records when Git reports staged deletion and an untracked file
    /// at the same path.
    pub entry_kind: StatusEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTreeFileDiff {
    /// The current path selected from a freshly-read working-tree status entry.
    pub path: String,
    pub old_path: Option<String>,
    pub patch: String,
    pub binary: bool,
    /// Successful results are never silently partial; the runtime errors at its limit.
    pub truncated: bool,
}
