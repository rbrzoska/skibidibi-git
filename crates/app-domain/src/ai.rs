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
    }
}
