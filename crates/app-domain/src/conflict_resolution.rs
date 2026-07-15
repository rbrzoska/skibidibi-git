use serde::{Deserialize, Serialize};

use crate::{RepositoryStatePrecondition, RepositoryStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictStageIdentity {
    pub oid: String,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictFileSummary {
    pub path: String,
    pub base: Option<ConflictStageIdentity>,
    pub ours: Option<ConflictStageIdentity>,
    pub theirs: Option<ConflictStageIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictListResult {
    pub files: Vec<ConflictFileSummary>,
    pub status: RepositoryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictVersion {
    pub identity: Option<ConflictStageIdentity>,
    pub content: Option<String>,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictFileDetailRequest {
    pub path: String,
    pub expected_base: Option<ConflictStageIdentity>,
    pub expected_ours: Option<ConflictStageIdentity>,
    pub expected_theirs: Option<ConflictStageIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictFileDetail {
    pub path: String,
    pub base: ConflictVersion,
    pub ours: ConflictVersion,
    pub theirs: ConflictVersion,
    pub working_content: Option<String>,
    pub working_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConflictResolution {
    Content { content: String },
    Ours,
    Theirs,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveConflictRequest {
    pub path: String,
    pub expected_base: Option<ConflictStageIdentity>,
    pub expected_ours: Option<ConflictStageIdentity>,
    pub expected_theirs: Option<ConflictStageIdentity>,
    pub resolution: ConflictResolution,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveConflictResult {
    pub resolved: bool,
    pub status: Option<RepositoryStatus>,
    pub error_message: Option<String>,
    pub mutation_may_have_occurred: bool,
}
