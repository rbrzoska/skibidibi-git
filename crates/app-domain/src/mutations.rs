use serde::{Deserialize, Serialize};

use crate::{RepositoryStatus, StatusEntryKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexAction {
    Stage,
    Unstage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "camelCase")]
pub enum ChangeSelection {
    All,
    Selected {
        entries: Vec<WorkingTreeEntrySelector>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTreeEntrySelector {
    pub path: String,
    pub old_path: Option<String>,
    pub entry_kind: StatusEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyIndexChangeRequest {
    pub action: IndexAction,
    pub selection: ChangeSelection,
    pub expected_head: Option<String>,
    pub expected_head_name: Option<String>,
    pub expected_detached: bool,
    pub expected_unborn: bool,
    pub expected_index_fingerprint: String,
    pub expected_worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyIndexChangeResult {
    pub changed: bool,
    pub status: RepositoryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCommitRequest {
    pub message: String,
    pub expected_head: Option<String>,
    pub expected_head_name: Option<String>,
    pub expected_detached: bool,
    pub expected_unborn: bool,
    pub expected_index_fingerprint: String,
    pub expected_worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCommitResult {
    pub oid: String,
    pub status: RepositoryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AmendCommitRequest {
    pub message: Option<String>,
    pub confirm_upstream_rewrite: bool,
    pub expected_head: Option<String>,
    pub expected_head_name: Option<String>,
    pub expected_detached: bool,
    pub expected_unborn: bool,
    pub expected_index_fingerprint: String,
    pub expected_worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AmendCommitState {
    Succeeded,
    /// Git may have changed HEAD, the index, or the worktree, but the requested amend could not be
    /// verified. The repository must be refreshed and inspected before retrying.
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AmendCommitResult {
    pub previous_oid: String,
    pub oid: Option<String>,
    pub status: Option<RepositoryStatus>,
    pub state: AmendCommitState,
    pub error_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_change_contract_serializes_with_a_tagged_scope() {
        let request = ApplyIndexChangeRequest {
            action: IndexAction::Stage,
            selection: ChangeSelection::Selected {
                entries: vec![WorkingTreeEntrySelector {
                    path: "src/zażółć.ts".to_owned(),
                    old_path: None,
                    entry_kind: StatusEntryKind::Untracked,
                }],
            },
            expected_head: Some("a".repeat(40)),
            expected_head_name: Some("main".to_owned()),
            expected_detached: false,
            expected_unborn: false,
            expected_index_fingerprint: "index-v1:0123".to_owned(),
            expected_worktree_fingerprint: "worktree-v1:4567".to_owned(),
        };

        let value = serde_json::to_value(request).expect("serialize mutation request");
        assert_eq!(value["action"], "stage");
        assert_eq!(value["selection"]["scope"], "selected");
        assert_eq!(value["selection"]["entries"][0]["entryKind"], "untracked");
    }

    #[test]
    fn amend_contract_serializes_no_edit_and_rewrite_confirmation_explicitly() {
        let request = AmendCommitRequest {
            message: None,
            confirm_upstream_rewrite: true,
            expected_head: Some("a".repeat(40)),
            expected_head_name: Some("main".to_owned()),
            expected_detached: false,
            expected_unborn: false,
            expected_index_fingerprint: "index-v1:0123".to_owned(),
            expected_worktree_fingerprint: "worktree-v1:4567".to_owned(),
        };

        let value = serde_json::to_value(request).expect("serialize amend request");
        assert!(value["message"].is_null());
        assert_eq!(value["confirmUpstreamRewrite"], true);
        assert_eq!(value["expectedHeadName"], "main");
    }
}
