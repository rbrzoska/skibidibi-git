use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryProvider {
    Local,
    #[serde(rename = "github")]
    GitHub,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryTransport {
    Local,
    Ssh,
    Https,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryAvailability {
    Unknown,
    Available,
    Missing,
    Inaccessible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntegrationHealthState {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntegrationHealthIssue {
    Authentication,
    Authorization,
    Network,
    NotFound,
    InvalidConfiguration,
    OperationFailed,
}

/// Structured instead of free-form so command output and remote URLs cannot leak secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationHealth {
    pub state: IntegrationHealthState,
    pub issue: Option<IntegrationHealthIssue>,
    pub checked_at: Option<i64>,
}

impl Default for IntegrationHealth {
    fn default() -> Self {
        Self {
            state: IntegrationHealthState::Unknown,
            issue: None,
            checked_at: None,
        }
    }
}

/// Non-secret coordinates used to identify a hosted repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedRepositoryIdentity {
    pub host: String,
    pub owner: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryWorktreeRole {
    Main,
    Linked,
    Bare,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepositoryRelationKind {
    Submodule,
}

/// A durable relationship between two independent Git repository groups.
///
/// Keeping this at group level means every remembered worktree of either
/// repository shares the same relationship without creating duplicate rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryGroupRelation {
    pub parent_repository_group_id: String,
    pub child_repository_group_id: String,
    pub kind: RepositoryRelationKind,
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryGitIdentity {
    /// Candidate id used only when the common directory has not been seen before.
    pub repository_group_id: String,
    pub canonical_common_dir: String,
    pub worktree_role: RepositoryWorktreeRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedRepository {
    pub id: String,
    pub canonical_path: String,
    pub display_name: String,
    pub provider: RepositoryProvider,
    pub transport: RepositoryTransport,
    pub hosted_identity: Option<HostedRepositoryIdentity>,
    pub repository_group_id: Option<String>,
    pub worktree_role: RepositoryWorktreeRole,
    pub availability: RepositoryAvailability,
    pub git_health: IntegrationHealth,
    pub github_health: IntegrationHealth,
    pub pinned: bool,
    pub open_count: u64,
    pub last_opened_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberRepositoryInput {
    /// Used only for inserts. An existing canonical path preserves its original id.
    pub id: String,
    pub path: String,
    pub display_name: String,
    pub provider: RepositoryProvider,
    pub transport: RepositoryTransport,
    pub hosted_identity: Option<HostedRepositoryIdentity>,
    pub git_identity: Option<RepositoryGitIdentity>,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryHealthUpdate {
    pub git: Option<IntegrationHealth>,
    pub github: Option<IntegrationHealth>,
    pub now: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_repository_group_contract_uses_camel_case() {
        let repository = RememberedRepository {
            id: "repo".into(),
            canonical_path: "/repo".into(),
            display_name: "Repo".into(),
            provider: RepositoryProvider::Local,
            transport: RepositoryTransport::Local,
            hosted_identity: None,
            repository_group_id: Some("group".into()),
            worktree_role: RepositoryWorktreeRole::Linked,
            availability: RepositoryAvailability::Available,
            git_health: IntegrationHealth::default(),
            github_health: IntegrationHealth::default(),
            pinned: false,
            open_count: 0,
            last_opened_at: None,
            created_at: 1,
            updated_at: 1,
        };

        let value = serde_json::to_value(repository).unwrap();
        assert_eq!(value["repositoryGroupId"], "group");
        assert_eq!(value["worktreeRole"], "linked");
    }

    #[test]
    fn repository_relation_contract_uses_camel_case() {
        let relation = RepositoryGroupRelation {
            parent_repository_group_id: "parent".into(),
            child_repository_group_id: "child".into(),
            kind: RepositoryRelationKind::Submodule,
            relative_path: "vendor/library".into(),
        };

        let value = serde_json::to_value(relation).unwrap();
        assert_eq!(value["parentRepositoryGroupId"], "parent");
        assert_eq!(value["childRepositoryGroupId"], "child");
        assert_eq!(value["kind"], "submodule");
        assert_eq!(value["relativePath"], "vendor/library");
    }
}
