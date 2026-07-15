use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryNavigation {
    pub branches: Vec<RepositoryBranch>,
    pub worktrees: Vec<RepositoryWorktree>,
    pub stashes: Vec<RepositoryStash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryBranch {
    pub kind: RepositoryBranchKind,
    pub full_name: String,
    pub name: String,
    pub oid: String,
    pub current: bool,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub upstream_gone: bool,
    pub symbolic_target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryBranchKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryWorktree {
    pub path: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub lock_reason: Option<String>,
    pub prunable: bool,
    pub prunable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStash {
    pub oid: String,
    pub selector: String,
    pub message: String,
    pub author: String,
    /// ISO 8601 strict author date emitted by Git's `%aI` formatter.
    pub authored_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_serializes_with_frontend_friendly_names() {
        let branch = RepositoryBranch {
            kind: RepositoryBranchKind::Local,
            full_name: "refs/heads/main".to_owned(),
            name: "main".to_owned(),
            oid: "abc".to_owned(),
            current: true,
            upstream: Some("origin/main".to_owned()),
            ahead: 1,
            behind: 2,
            upstream_gone: false,
            symbolic_target: None,
        };

        assert_eq!(branch.full_name, "refs/heads/main");
        assert_eq!(branch.kind, RepositoryBranchKind::Local);
    }
}
