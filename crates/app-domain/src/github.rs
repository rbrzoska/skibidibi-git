use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitHubAuthKind {
    PersonalAccessToken,
    OAuthDevice,
    GitHubCli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitHubAccountState {
    Unknown,
    Connected,
    AuthenticationRequired,
    Unavailable,
}

/// Public state of a GitHub Device Flow session. Secret provider codes never cross the UI
/// boundary; the native layer identifies a session with an application-generated `flow_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitHubDeviceFlowState {
    Pending,
    Authorized,
    Expired,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubDeviceFlowStart {
    pub flow_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: i64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubDeviceFlowPoll {
    pub state: GitHubDeviceFlowState,
    pub next_poll_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubAccountSummary {
    pub id: String,
    pub host: String,
    pub login: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub auth_kind: GitHubAuthKind,
    pub scopes: Vec<String>,
    pub state: GitHubAccountState,
    pub last_validated_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubUser {
    /// Some GraphQL actors (for example deleted users or integrations) have no database id.
    pub id: Option<u64>,
    pub login: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRepository {
    /// GitHub's stable numeric database id represented as a string to avoid
    /// precision loss when the DTO crosses the JavaScript boundary.
    pub id: String,
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub updated_at: String,
    pub https_clone_url: String,
    pub ssh_clone_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestMergeability {
    Unknown,
    Mergeable,
    Conflicting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestSummary {
    pub number: u64,
    pub title: String,
    pub state: PullRequestState,
    pub draft: bool,
    pub author: Option<GitHubUser>,
    pub head_ref: String,
    pub base_ref: String,
    pub html_url: String,
    pub updated_at: String,
    pub comment_count: u64,
    pub approval_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub summary: PullRequestSummary,
    pub body_markdown: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub mergeability: PullRequestMergeability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestFile {
    pub filename: String,
    pub previous_filename: Option<String>,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub changes: u64,
    pub patch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueComment {
    pub id: u64,
    pub author: Option<GitHubUser>,
    pub body_markdown: String,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewCommentSide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub node_id: String,
    pub author: Option<GitHubUser>,
    pub body_markdown: String,
    pub path: Option<String>,
    pub line: Option<u64>,
    pub side: Option<ReviewCommentSide>,
    pub diff_hunk: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThread {
    pub node_id: String,
    pub resolved: bool,
    pub outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub side: Option<ReviewCommentSide>,
    pub comments: Vec<ReviewComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRateLimit {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub reset_at: Option<i64>,
    pub retry_after_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPage<T> {
    pub items: Vec<T>,
    /// Opaque provider cursor. Callers must not construct or modify it.
    pub next_cursor: Option<String>,
    pub rate_limit: GitHubRateLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubApiResult<T> {
    pub value: T,
    pub rate_limit: GitHubRateLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPatValidation {
    pub user: GitHubUser,
    pub scopes: Vec<String>,
    pub rate_limit: GitHubRateLimit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_flow_contract_is_camel_case_and_contains_no_provider_secret() {
        let value = serde_json::to_value(GitHubDeviceFlowStart {
            flow_id: "local-flow-id".to_owned(),
            user_code: "ABCD-1234".to_owned(),
            verification_uri: "https://github.com/login/device".to_owned(),
            expires_at: 1_800_000_000,
            interval_seconds: 5,
        })
        .unwrap();

        assert_eq!(value["flowId"], "local-flow-id");
        assert_eq!(value["userCode"], "ABCD-1234");
        assert_eq!(value["intervalSeconds"], 5);
        assert!(value.get("deviceCode").is_none());
        assert!(value.get("accessToken").is_none());
    }

    #[test]
    fn github_cli_auth_kind_has_a_stable_frontend_name() {
        assert_eq!(
            serde_json::to_string(&GitHubAuthKind::GitHubCli).unwrap(),
            "\"gitHubCli\""
        );
    }
}
