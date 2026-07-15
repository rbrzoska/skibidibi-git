use std::{path::Path, str};

use app_domain::{HostedRepositoryIdentity, RepositoryProvider, RepositoryTransport};
use thiserror::Error;

use crate::{GitExecutor, GitOutput, GitRunError};

const HEAD_ARGUMENTS: &[&str] = &["symbolic-ref", "--quiet", "--short", "HEAD"];
const REMOTES_ARGUMENTS: &[&str] = &["remote"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMetadata {
    pub default_remote: Option<String>,
    pub fetch_url: Option<String>,
    pub push_url: Option<String>,
    pub provider: RepositoryProvider,
    pub transport: RepositoryTransport,
    pub hosted_identity: Option<HostedRepositoryIdentity>,
}

impl RepositoryMetadata {
    fn local() -> Self {
        Self {
            default_remote: None,
            fetch_url: None,
            push_url: None,
            provider: RepositoryProvider::Local,
            transport: RepositoryTransport::Local,
            hosted_identity: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum RemoteMetadataError {
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error("git returned non-UTF-8 remote metadata")]
    InvalidUtf8,
    #[error("git returned an unsafe remote URL")]
    UnsafeUrl,
}

pub fn discover_repository_metadata(
    executor: &impl GitExecutor,
    repository: &Path,
) -> Result<RepositoryMetadata, RemoteMetadataError> {
    let head = optional_output(executor.execute(repository, HEAD_ARGUMENTS))?
        .map(output_text)
        .transpose()?
        .and_then(non_empty);

    let configured_remote = if let Some(head) = head.as_deref() {
        let key = format!("branch.{head}.remote");
        optional_output(executor.execute(repository, &["config", "--get", &key]))?
            .map(output_text)
            .transpose()?
            .and_then(non_empty)
            .filter(|remote| remote != ".")
    } else {
        None
    };

    let remotes = output_text(executor.execute(repository, REMOTES_ARGUMENTS)?)?
        .lines()
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let Some(default_remote) = select_default_remote(configured_remote.as_deref(), &remotes) else {
        return Ok(RepositoryMetadata::local());
    };

    let fetch_output =
        executor.execute(repository, &["remote", "get-url", "--", &default_remote])?;
    let fetch_url = sanitize_url(&output_text(fetch_output)?)?;
    let push_output = executor.execute(
        repository,
        &["remote", "get-url", "--push", "--", &default_remote],
    )?;
    let push_url = sanitize_url(&output_text(push_output)?)?;
    let (provider, transport, hosted_identity) = classify_remote(&fetch_url);

    Ok(RepositoryMetadata {
        default_remote: Some(default_remote),
        fetch_url: Some(fetch_url),
        push_url: Some(push_url),
        provider,
        transport,
        hosted_identity,
    })
}

fn optional_output(
    result: Result<GitOutput, GitRunError>,
) -> Result<Option<GitOutput>, GitRunError> {
    match result {
        Ok(output) => Ok(Some(output)),
        Err(GitRunError::Unsuccessful { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn output_text(output: GitOutput) -> Result<String, RemoteMetadataError> {
    String::from_utf8(output.stdout).map_err(|_| RemoteMetadataError::InvalidUtf8)
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn select_default_remote(configured: Option<&str>, remotes: &[String]) -> Option<String> {
    configured
        .filter(|candidate| remotes.iter().any(|remote| remote == candidate))
        .map(str::to_owned)
        .or_else(|| {
            remotes
                .iter()
                .find(|remote| remote.as_str() == "origin")
                .cloned()
        })
        .or_else(|| remotes.first().cloned())
}

fn sanitize_url(value: &str) -> Result<String, RemoteMetadataError> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(RemoteMetadataError::UnsafeUrl);
    }

    let without_parameters = value.split_once(['?', '#']).map_or(value, |(safe, _)| safe);
    let Some((scheme, remainder)) = without_parameters.split_once("://") else {
        return Ok(without_parameters.to_owned());
    };
    let (authority, path) = remainder
        .split_once('/')
        .map_or((remainder, ""), |(authority, path)| (authority, path));
    let safe_authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if safe_authority.is_empty() {
        return Err(RemoteMetadataError::UnsafeUrl);
    }

    if path.is_empty() {
        Ok(format!("{scheme}://{safe_authority}"))
    } else {
        Ok(format!("{scheme}://{safe_authority}/{path}"))
    }
}

fn classify_remote(
    url: &str,
) -> (
    RepositoryProvider,
    RepositoryTransport,
    Option<HostedRepositoryIdentity>,
) {
    let (transport, host, path) = if let Some((scheme, remainder)) = url.split_once("://") {
        let transport = match scheme.to_ascii_lowercase().as_str() {
            "ssh" => RepositoryTransport::Ssh,
            "https" => RepositoryTransport::Https,
            "file" => RepositoryTransport::Local,
            _ => RepositoryTransport::Other,
        };
        let (authority, path) = remainder
            .split_once('/')
            .map_or((remainder, ""), |(host, path)| (host, path));
        (
            transport,
            authority.rsplit('@').next().unwrap_or(authority),
            path,
        )
    } else if let Some((authority, path)) = scp_parts(url) {
        (
            RepositoryTransport::Ssh,
            authority.rsplit('@').next().unwrap_or(authority),
            path,
        )
    } else {
        (RepositoryTransport::Local, "", url)
    };

    let host_without_port = host.split(':').next().unwrap_or(host);
    if host_without_port.eq_ignore_ascii_case("github.com") {
        let mut segments = path.trim_matches('/').split('/');
        let owner = segments.next().unwrap_or_default();
        let name = segments.next().unwrap_or_default().trim_end_matches(".git");
        if !owner.is_empty() && !name.is_empty() && segments.next().is_none() {
            return (
                RepositoryProvider::GitHub,
                transport,
                Some(HostedRepositoryIdentity {
                    host: "github.com".to_owned(),
                    owner: owner.to_owned(),
                    name: name.to_owned(),
                }),
            );
        }
    }

    let provider = if transport == RepositoryTransport::Local {
        RepositoryProvider::Local
    } else {
        RepositoryProvider::Other
    };
    (provider, transport, None)
}

fn scp_parts(url: &str) -> Option<(&str, &str)> {
    let (authority, path) = url.split_once(':')?;
    if authority.len() == 1 && authority.as_bytes()[0].is_ascii_alphabetic() {
        return None;
    }
    (!authority.is_empty() && !path.is_empty()).then_some((authority, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_remote_using_branch_then_origin_then_first_priority() {
        let cases = [
            (
                Some("upstream"),
                &["origin", "upstream"][..],
                Some("upstream"),
            ),
            (Some("missing"), &["first", "origin"][..], Some("origin")),
            (None, &["first", "second"][..], Some("first")),
            (None, &[][..], None),
        ];

        for (configured, remotes, expected) in cases {
            let remotes = remotes
                .iter()
                .map(|remote| (*remote).to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                select_default_remote(configured, &remotes).as_deref(),
                expected
            );
        }
    }

    #[test]
    fn classifies_supported_remote_forms() {
        let cases = [
            (
                "git@github.com:owner/repository.git",
                RepositoryProvider::GitHub,
                RepositoryTransport::Ssh,
                Some(("owner", "repository")),
            ),
            (
                "ssh://git@github.com/owner/repository.git",
                RepositoryProvider::GitHub,
                RepositoryTransport::Ssh,
                Some(("owner", "repository")),
            ),
            (
                "https://github.com/owner/repository.git",
                RepositoryProvider::GitHub,
                RepositoryTransport::Https,
                Some(("owner", "repository")),
            ),
            (
                "https://gitlab.com/group/repository.git",
                RepositoryProvider::Other,
                RepositoryTransport::Https,
                None,
            ),
            (
                "../repository",
                RepositoryProvider::Local,
                RepositoryTransport::Local,
                None,
            ),
        ];

        for (url, expected_provider, expected_transport, expected_identity) in cases {
            let (provider, transport, identity) = classify_remote(url);
            assert_eq!(provider, expected_provider, "{url}");
            assert_eq!(transport, expected_transport, "{url}");
            assert_eq!(
                identity
                    .as_ref()
                    .map(|identity| (identity.owner.as_str(), identity.name.as_str())),
                expected_identity,
                "{url}"
            );
        }
    }

    #[test]
    fn sanitizes_credentials_and_query_parameters() {
        let cases = [
            (
                "https://oauth-token:secret@github.com/owner/repo.git?token=secret#fragment",
                "https://github.com/owner/repo.git",
            ),
            (
                "ssh://private-user@github.com/owner/repo.git?unsafe=yes",
                "ssh://github.com/owner/repo.git",
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(sanitize_url(input).expect("safe URL"), expected);
        }
    }

    #[test]
    fn rejects_control_characters_in_urls() {
        assert!(matches!(
            sanitize_url("https://github.com/owner/repo.git\nsecret"),
            Err(RemoteMetadataError::UnsafeUrl)
        ));
    }
}
