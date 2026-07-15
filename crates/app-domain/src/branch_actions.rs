use serde::{Deserialize, Serialize};

use crate::AutoStashOutcome;

/// Identifies an existing local branch without relying on Git's DWIM resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchBranchRequest {
    /// A fully-qualified local ref, for example `refs/heads/feature/safe`.
    pub full_name: String,
    /// The target OID observed when the operation was prepared.
    pub expected_oid: String,
    pub stash_on_dirty: bool,
    pub stash_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchBranchResult {
    pub full_name: String,
    pub name: String,
    pub head: String,
    /// False when the requested branch was already checked out in this worktree.
    pub changed: bool,
    pub stash_created: bool,
    /// Whether Git switched to the requested branch. Restore conflicts do not change this value.
    pub operation_succeeded: bool,
    pub operation_error: Option<String>,
    pub auto_stash: AutoStashOutcome,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_keeps_full_ref_and_no_op_state() {
        let request = SwitchBranchRequest {
            full_name: "refs/heads/feature/safe".to_owned(),
            expected_oid: "0123456789012345678901234567890123456789".to_owned(),
            stash_on_dirty: false,
            stash_message: None,
        };
        let value = SwitchBranchResult {
            full_name: "refs/heads/feature/safe".to_owned(),
            name: "feature/safe".to_owned(),
            head: "0123456789012345678901234567890123456789".to_owned(),
            changed: false,
            stash_created: false,
            operation_succeeded: true,
            operation_error: None,
            auto_stash: AutoStashOutcome::not_requested(),
        };

        assert_eq!(value.full_name, "refs/heads/feature/safe");
        assert_eq!(value.name, "feature/safe");
        assert!(!value.changed);
        let request = serde_json::to_value(request).expect("serialize switch request");
        assert_eq!(
            request["expectedOid"],
            "0123456789012345678901234567890123456789"
        );
    }
}
