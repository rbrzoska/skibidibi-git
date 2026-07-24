use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiProvider {
    Codex,
    Claude,
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCliStatus {
    pub provider: AiProvider,
    pub display_name: String,
    pub available: bool,
    pub version: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCliStatuses {
    pub statuses: Vec<AiCliStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGenerateCommitMessageRequest {
    pub repository_id: String,
    pub provider: AiProvider,
    pub prompt_template: String,
    pub expected_head: Option<String>,
    pub index_fingerprint: String,
    pub worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGenerateCommitMessageResult {
    pub message: String,
    pub index_fingerprint: String,
    pub worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskReviewPreflightRequest {
    pub repository_id: String,
    pub target_full_name: String,
    pub target_oid: String,
    pub expected_head: String,
    pub index_fingerprint: String,
    pub worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskReviewPreflightResult {
    pub branch: String,
    pub head: String,
    pub target_full_name: String,
    pub target_oid: String,
    pub merge_base: String,
    pub target_merged: bool,
    pub uncommitted_files: usize,
    pub changed_files: usize,
    pub my_commits: usize,
    pub index_fingerprint: String,
    pub worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGenerateTaskReviewRequest {
    pub repository_id: String,
    pub provider: AiProvider,
    pub prompt_template: String,
    pub target_full_name: String,
    pub target_oid: String,
    pub expected_head: String,
    pub index_fingerprint: String,
    pub worktree_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCodeReviewSummary {
    pub id: String,
    pub repository_id: String,
    pub repository_name: String,
    pub branch: String,
    pub target_branch: String,
    pub provider: AiProvider,
    pub created_at_ms: u64,
    pub changed_files: usize,
    pub my_commits: usize,
    pub uncommitted_files: usize,
    pub markdown_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCodeReviewDocument {
    pub summary: AiCodeReviewSummary,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCodeReviewList {
    pub reviews: Vec<AiCodeReviewSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_contract_uses_camel_case_and_stable_provider_ids() {
        let request = serde_json::from_value::<AiGenerateCommitMessageRequest>(serde_json::json!({
            "repositoryId": "repo",
            "provider": "codex",
            "promptTemplate": "Write a commit subject",
            "expectedHead": null,
            "indexFingerprint": "index",
            "worktreeFingerprint": "worktree"
        }))
        .unwrap();
        assert_eq!(request.provider, AiProvider::Codex);
        let status = AiCliStatus {
            provider: AiProvider::Cursor,
            display_name: "Cursor Agent".to_owned(),
            available: true,
            version: Some("1.0".to_owned()),
            detail: None,
        };
        assert_eq!(serde_json::to_value(status).unwrap()["provider"], "cursor");

        let preflight = serde_json::from_value::<AiTaskReviewPreflightRequest>(serde_json::json!({
            "repositoryId": "repo",
            "targetFullName": "refs/heads/release/2026.7",
            "targetOid": "2222222222222222222222222222222222222222",
            "expectedHead": "1111111111111111111111111111111111111111",
            "indexFingerprint": "index",
            "worktreeFingerprint": "worktree"
        }))
        .unwrap();
        assert_eq!(preflight.target_full_name, "refs/heads/release/2026.7");

        let summary = AiCodeReviewSummary {
            id: "review-1".to_owned(),
            repository_id: "repo".to_owned(),
            repository_name: "project".to_owned(),
            branch: "feature/task".to_owned(),
            target_branch: "refs/heads/main".to_owned(),
            provider: AiProvider::Codex,
            created_at_ms: 1,
            changed_files: 3,
            my_commits: 2,
            uncommitted_files: 1,
            markdown_file: "/data/review-1.md".to_owned(),
        };
        let value = serde_json::to_value(summary).unwrap();
        assert_eq!(value["targetBranch"], "refs/heads/main");
        assert_eq!(value["myCommits"], 2);
    }
}
