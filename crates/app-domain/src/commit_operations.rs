use serde::{Deserialize, Serialize};

use crate::{RepositoryStatePrecondition, RepositoryStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitOperationRequest {
    pub target_oid: String,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCommitRequest {
    pub target_oid: String,
    pub mode: ResetMode,
    /// Required in addition to the renderer's destructive confirmation.
    pub confirm_hard_reset: bool,
    pub precondition: RepositoryStatePrecondition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommitOperationState {
    Succeeded,
    Conflicted,
    /// The command was attempted, but its exact effect could not be verified. Callers must force a
    /// full refresh before enabling another mutation.
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitOperationResult {
    pub state: CommitOperationState,
    pub target_oid: String,
    pub head_before: String,
    pub head_after: Option<String>,
    /// Present for successful and conflicted outcomes. It is absent only when post-operation
    /// inspection itself failed.
    pub status: Option<RepositoryStatus>,
    pub error_message: Option<String>,
    /// Always true for a returned result because results are produced only after invoking a
    /// command capable of mutating the repository.
    pub mutation_may_have_occurred: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_reset_confirmation_is_explicit_in_the_wire_contract() {
        let request = ResetCommitRequest {
            target_oid: "a".repeat(40),
            mode: ResetMode::Hard,
            confirm_hard_reset: true,
            precondition: RepositoryStatePrecondition {
                expected_head: Some("b".repeat(40)),
                expected_head_name: Some("main".to_owned()),
                expected_detached: false,
                expected_unborn: false,
                expected_index_fingerprint: "index-v1:1234".to_owned(),
                expected_worktree_fingerprint: "worktree-v1:5678".to_owned(),
            },
        };

        let value = serde_json::to_value(request).expect("serialize reset request");
        assert_eq!(value["mode"], "hard");
        assert_eq!(value["confirmHardReset"], true);
        assert_eq!(value["targetOid"], "a".repeat(40));
    }
}
