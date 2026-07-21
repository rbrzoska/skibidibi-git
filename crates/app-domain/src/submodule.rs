use serde::{Deserialize, Serialize};

/// Immediate submodule state for a repository. Nested submodules are intentionally not expanded:
/// opening one of these paths creates a separate repository context instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySubmodules {
    pub submodules: Vec<RepositorySubmodule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySubmodule {
    /// The configured submodule name, falling back to its repository-relative path when the
    /// optional `.gitmodules` metadata is absent or unusable.
    pub name: String,
    /// A repository-relative, normalized path. Absolute paths are never exposed here.
    pub path: String,
    /// Sanitized configured URL. Credentials, queries, and fragments are omitted.
    pub url: Option<String>,
    /// The commit recorded by the parent repository's index.
    pub expected_oid: Option<String>,
    /// The checked-out child `HEAD`, when the child is initialized and inspectable.
    pub current_oid: Option<String>,
    pub present: bool,
    pub initialized: bool,
    pub commit_state: SubmoduleCommitState,
    pub worktree_state: SubmoduleWorktreeState,
    pub change_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubmoduleCommitState {
    AtExpected,
    Different,
    Unavailable,
    Conflicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubmoduleWorktreeState {
    Clean,
    Modified,
    Untracked,
    ModifiedAndUntracked,
    Conflicted,
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_submodule_contract_in_camel_case() {
        let value = serde_json::to_value(RepositorySubmodule {
            name: "client".into(),
            path: "modules/client".into(),
            url: Some("https://github.example/acme/client.git".into()),
            expected_oid: Some("a".repeat(40)),
            current_oid: Some("b".repeat(40)),
            present: true,
            initialized: true,
            commit_state: SubmoduleCommitState::Different,
            worktree_state: SubmoduleWorktreeState::ModifiedAndUntracked,
            change_count: 2,
        })
        .expect("serialize");

        assert_eq!(value["expectedOid"], "a".repeat(40));
        assert_eq!(value["commitState"], "different");
        assert_eq!(value["worktreeState"], "modifiedAndUntracked");
    }
}
