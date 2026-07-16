use serde::{Deserialize, Serialize};

/// Selects an exact commit from which a new local branch will be created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BranchCreationSource {
    Current {
        expected_oid: String,
    },
    Commit {
        oid: String,
    },
    RemoteTracking {
        full_name: String,
        expected_oid: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchRequest {
    /// A short local branch name, for example `feature/safe`. When omitted for a
    /// remote-tracking source, the runtime derives it from the configured remote.
    pub name: Option<String>,
    pub source: BranchCreationSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchResult {
    pub full_name: String,
    pub name: String,
    pub head: String,
    /// The short upstream name used by repository navigation, for example `origin/main`.
    pub upstream: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_tracking_source_uses_a_tagged_frontend_contract() {
        let request = CreateBranchRequest {
            name: Some("feature/safe".to_owned()),
            source: BranchCreationSource::RemoteTracking {
                full_name: "refs/remotes/origin/main".to_owned(),
                expected_oid: "0123456789012345678901234567890123456789".to_owned(),
            },
        };

        let value = serde_json::to_value(request).expect("serialize branch request");
        assert_eq!(value["name"], "feature/safe");
        assert_eq!(value["source"]["kind"], "remoteTracking");
        assert_eq!(value["source"]["fullName"], "refs/remotes/origin/main");
        assert_eq!(
            value["source"]["expectedOid"],
            "0123456789012345678901234567890123456789"
        );
    }
}
