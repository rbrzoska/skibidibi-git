use serde::{Deserialize, Serialize};

/// Describes a shell-free clone into one direct child of an existing directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneRepositoryRequest {
    pub source_url: String,
    pub destination_parent: String,
    pub directory_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneRepositoryResult {
    pub repository_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_request_uses_the_frontend_camel_case_contract() {
        let request = CloneRepositoryRequest {
            source_url: "https://github.com/example/project.git".to_owned(),
            destination_parent: "/tmp/projects".to_owned(),
            directory_name: "project".to_owned(),
        };

        let value = serde_json::to_value(request).expect("serialize clone request");
        assert_eq!(value["sourceUrl"], "https://github.com/example/project.git");
        assert_eq!(value["destinationParent"], "/tmp/projects");
        assert_eq!(value["directoryName"], "project");
    }
}
