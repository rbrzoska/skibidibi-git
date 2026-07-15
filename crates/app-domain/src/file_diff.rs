use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffRequest {
    pub oid: String,
    pub path: String,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub oid: String,
    /// The path selected by the user. It is kept separate from paths rendered by Git in the patch.
    pub path: String,
    pub patch: String,
    pub binary: bool,
    /// Successful results are never silently partial; the runtime returns an error at its limit.
    pub truncated: bool,
}
