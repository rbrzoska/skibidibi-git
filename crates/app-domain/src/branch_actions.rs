use serde::{Deserialize, Serialize};

/// Identifies an existing local branch without relying on Git's DWIM resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchBranchRequest {
    /// A fully-qualified local ref, for example `refs/heads/feature/safe`.
    pub full_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchBranchResult {
    pub full_name: String,
    pub name: String,
    pub head: String,
    /// False when the requested branch was already checked out in this worktree.
    pub changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_keeps_full_ref_and_no_op_state() {
        let value = SwitchBranchResult {
            full_name: "refs/heads/feature/safe".to_owned(),
            name: "feature/safe".to_owned(),
            head: "0123456789012345678901234567890123456789".to_owned(),
            changed: false,
        };

        assert_eq!(value.full_name, "refs/heads/feature/safe");
        assert_eq!(value.name, "feature/safe");
        assert!(!value.changed);
    }
}
