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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberedRepository {
    pub id: String,
    pub canonical_path: String,
    pub display_name: String,
    pub provider: RepositoryProvider,
    pub transport: RepositoryTransport,
    pub hosted_identity: Option<HostedRepositoryIdentity>,
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
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryHealthUpdate {
    pub git: Option<IntegrationHealth>,
    pub github: Option<IntegrationHealth>,
    pub now: i64,
}
