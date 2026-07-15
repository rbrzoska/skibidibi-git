use serde::{Deserialize, Serialize};

use crate::{AutoStashOptions, AutoStashOutcome, RepositoryStatePrecondition, RepositoryStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullStrategy {
    FfIfPossible,
    FfOnly,
    Rebase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub strategy: PullStrategy,
    pub auto_stash: Option<AutoStashOptions>,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullOperationState {
    Succeeded,
    Conflicted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    pub state: PullOperationState,
    pub head_before: String,
    pub head_after: Option<String>,
    pub status: Option<RepositoryStatus>,
    pub auto_stash: AutoStashOutcome,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PushReadiness {
    NoUpstream,
    UpToDate,
    Ready,
    Behind,
    Diverged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushAnalysis {
    pub branch: String,
    pub head: String,
    pub upstream: Option<String>,
    pub remote: Option<String>,
    pub remote_ref: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub readiness: PushReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PushTarget {
    Configured {
        expected_upstream: String,
    },
    SetUpstream {
        remote: String,
        remote_branch: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub target: PushTarget,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub pushed: bool,
    pub analysis: PushAnalysis,
    pub status: RepositoryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUpstreamRequest {
    pub remote_full_name: String,
    pub expected_oid: String,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetUpstreamResult {
    pub upstream: String,
    pub status: RepositoryStatus,
}
