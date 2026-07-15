use std::{ffi::OsString, path::Path, time::Duration};

use app_domain::{
    BranchCreationSource, CreateBranchRequest, CreateBranchResult, RepositoryBranch,
    RepositoryBranchKind,
};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, NavigationParseError,
    parse_branch_records,
};
use thiserror::Error;

use super::RepositoryRuntime;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const QUERY_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const ACTION_OUTPUT_LIMIT: usize = 256 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const MAX_FULL_REF_BYTES: usize = 1024;
const BRANCH_FORMAT: &str = "--format=%(refname)%00%(refname:short)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(symref)%00";

pub trait BranchCreationGitExecutor: Send + Sync {
    fn query_branches(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn resolve_head(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn verify_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
    fn create_ref(
        &self,
        repository: &Path,
        full_name: &str,
        oid: &str,
    ) -> Result<GitOutput, GitRunError>;
    fn set_upstream(
        &self,
        repository: &Path,
        branch_name: &str,
        upstream_full_name: &str,
    ) -> Result<GitOutput, GitRunError>;
    fn delete_ref_if_unchanged(
        &self,
        repository: &Path,
        full_name: &str,
        expected_oid: &str,
    ) -> Result<GitOutput, GitRunError>;
}

impl BranchCreationGitExecutor for GitRunner {
    fn query_branches(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["for-each-ref", BRANCH_FORMAT, "refs/heads", "refs/remotes"],
            )
            .with_output_limits(QUERY_OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn resolve_head(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["rev-parse", "--verify", "--quiet", "HEAD"],
            )
            .with_output_limits(128, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn verify_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["show", "-s", "--format=%H%x00%T", oid, "--"],
            )
            .with_output_limits(256, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn create_ref(
        &self,
        repository: &Path,
        full_name: &str,
        oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        let zero_oid = "0".repeat(oid.len());
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    OsString::from("update-ref"),
                    OsString::from(full_name),
                    OsString::from(oid),
                    OsString::from(zero_oid),
                ],
            )
            .with_output_limits(ACTION_OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn set_upstream(
        &self,
        repository: &Path,
        branch_name: &str,
        upstream_full_name: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    OsString::from("branch"),
                    OsString::from(format!("--set-upstream-to={upstream_full_name}")),
                    OsString::from("--"),
                    OsString::from(branch_name),
                ],
            )
            .with_output_limits(ACTION_OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn delete_ref_if_unchanged(
        &self,
        repository: &Path,
        full_name: &str,
        expected_oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["update-ref", "-d", full_name, expected_oid],
            )
            .with_output_limits(ACTION_OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum BranchCreationError {
    #[error("branch name is invalid")]
    InvalidBranchName,
    #[error("source object ID is invalid")]
    InvalidObjectId,
    #[error("a local branch with this name already exists")]
    BranchAlreadyExists,
    #[error("the current HEAD changed since the request was prepared")]
    CurrentHeadChanged,
    #[error("the selected object is not an inspectable commit")]
    SourceNotCommit,
    #[error("the remote-tracking branch does not exist")]
    RemoteBranchNotFound,
    #[error("the remote-tracking branch changed since the request was prepared")]
    RemoteBranchChanged,
    #[error("symbolic remote refs cannot be used as a branch source")]
    SymbolicRemoteNotAllowed,
    #[error("tracking setup failed after creating the branch; the new ref was removed: {0}")]
    TrackingSetup(GitRunError),
    #[error(
        "tracking setup failed and cleanup could not remove the created branch; manual cleanup is required (tracking: {tracking}; cleanup: {cleanup})"
    )]
    TrackingSetupCleanupFailed { tracking: String, cleanup: String },
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    InvalidOutput(#[from] NavigationParseError),
}

impl BranchCreationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidBranchName | Self::InvalidObjectId => "invalidRequest",
            Self::BranchAlreadyExists => "branchAlreadyExists",
            Self::CurrentHeadChanged | Self::RemoteBranchChanged => "sourceChanged",
            Self::SourceNotCommit | Self::RemoteBranchNotFound | Self::SymbolicRemoteNotAllowed => {
                "sourceUnavailable"
            }
            Self::TrackingSetup(_) => "trackingSetupRejected",
            Self::TrackingSetupCleanupFailed { .. } => "manualCleanupRequired",
            Self::Git(_) | Self::InvalidOutput(_) => "gitRejected",
        }
    }
}

impl<E> RepositoryRuntime<E>
where
    E: BranchCreationGitExecutor,
{
    pub fn create_branch(
        &self,
        repository: &Path,
        request: &CreateBranchRequest,
    ) -> Result<CreateBranchResult, BranchCreationError> {
        let full_name = validated_full_name(&request.name)?;
        let output = self.executor.query_branches(repository)?;
        let branches = parse_branch_records(&output.stdout)?;
        if branches.iter().any(|branch| branch.full_name == full_name) {
            return Err(BranchCreationError::BranchAlreadyExists);
        }

        let (oid, upstream) = self.resolve_source(repository, &branches, &request.source)?;
        verify_commit_output(&self.executor.verify_commit(repository, &oid)?.stdout, &oid)?;
        self.executor.create_ref(repository, &full_name, &oid)?;

        let upstream = if let Some(upstream) = upstream {
            if let Err(tracking) =
                self.executor
                    .set_upstream(repository, &request.name, &upstream.full_name)
            {
                return match self
                    .executor
                    .delete_ref_if_unchanged(repository, &full_name, &oid)
                {
                    Ok(_) => Err(BranchCreationError::TrackingSetup(tracking)),
                    Err(cleanup) => Err(BranchCreationError::TrackingSetupCleanupFailed {
                        tracking: tracking.to_string(),
                        cleanup: cleanup.to_string(),
                    }),
                };
            }
            Some(upstream.name)
        } else {
            None
        };

        Ok(CreateBranchResult {
            full_name,
            name: request.name.clone(),
            head: oid,
            upstream,
        })
    }

    fn resolve_source(
        &self,
        repository: &Path,
        branches: &[RepositoryBranch],
        source: &BranchCreationSource,
    ) -> Result<(String, Option<RepositoryBranch>), BranchCreationError> {
        match source {
            BranchCreationSource::Current { expected_oid } => {
                let expected_oid = validated_oid(expected_oid)?;
                let output = self.executor.resolve_head(repository)?;
                let actual = parse_single_oid(&output.stdout)
                    .ok_or(BranchCreationError::CurrentHeadChanged)?;
                if actual != expected_oid {
                    return Err(BranchCreationError::CurrentHeadChanged);
                }
                Ok((actual, None))
            }
            BranchCreationSource::Commit { oid } => Ok((validated_oid(oid)?, None)),
            BranchCreationSource::RemoteTracking {
                full_name,
                expected_oid,
            } => {
                let expected_oid = validated_oid(expected_oid)?;
                let remote = branches
                    .iter()
                    .find(|branch| branch.full_name == *full_name)
                    .filter(|branch| branch.kind == RepositoryBranchKind::Remote)
                    .ok_or(BranchCreationError::RemoteBranchNotFound)?;
                if remote.symbolic_target.is_some() {
                    return Err(BranchCreationError::SymbolicRemoteNotAllowed);
                }
                if remote.oid != expected_oid {
                    return Err(BranchCreationError::RemoteBranchChanged);
                }
                Ok((expected_oid, Some(remote.clone())))
            }
        }
    }
}

fn validated_full_name(name: &str) -> Result<String, BranchCreationError> {
    let full_name = format!("refs/heads/{name}");
    if full_name.len() > MAX_FULL_REF_BYTES || !is_valid_branch_name(name) {
        return Err(BranchCreationError::InvalidBranchName);
    }
    Ok(full_name)
}

fn validated_oid(oid: &str) -> Result<String, BranchCreationError> {
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BranchCreationError::InvalidObjectId);
    }
    Ok(oid.to_ascii_lowercase())
}

fn parse_single_oid(output: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(output).ok()?.trim();
    validated_oid(value).ok()
}

fn verify_commit_output(output: &[u8], expected_oid: &str) -> Result<(), BranchCreationError> {
    let text = std::str::from_utf8(output).map_err(|_| BranchCreationError::SourceNotCommit)?;
    let mut fields = text.trim_end().split('\0');
    let commit = fields.next().and_then(parse_verified_oid);
    let tree = fields.next().and_then(parse_verified_oid);
    if fields.next().is_some() || commit.as_deref() != Some(expected_oid) || tree.is_none() {
        return Err(BranchCreationError::SourceNotCommit);
    }
    Ok(())
}

fn parse_verified_oid(value: &str) -> Option<String> {
    validated_oid(value).ok()
}

fn is_valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name != "@"
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name.contains("//")
        && !name
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
        && !name.bytes().any(|byte| {
            matches!(
                byte,
                0x00..=0x20 | 0x7f | b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\'
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_nested_names_and_rejects_option_or_ref_syntax() {
        assert_eq!(
            validated_full_name("feature/safe").unwrap(),
            "refs/heads/feature/safe"
        );
        for invalid in [
            "", "@", "-force", ".hidden", "a..b", "a@{b", "a//b", "a.lock", "a b", "a\\b",
        ] {
            assert!(
                validated_full_name(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn accepts_sha_one_and_sha_256_but_not_partial_object_ids() {
        assert!(validated_oid(&"a".repeat(40)).is_ok());
        assert!(validated_oid(&"B".repeat(64)).is_ok());
        assert!(validated_oid(&"a".repeat(39)).is_err());
        assert!(validated_oid(&format!("{}g", "a".repeat(39))).is_err());
    }

    #[test]
    fn commit_verification_requires_the_exact_commit_and_a_tree() {
        let oid = "a".repeat(40);
        let tree = "b".repeat(40);
        verify_commit_output(format!("{oid}\0{tree}\n").as_bytes(), &oid).unwrap();
        assert!(verify_commit_output(format!("{tree}\0{oid}\n").as_bytes(), &oid).is_err());
        assert!(verify_commit_output(oid.as_bytes(), &oid).is_err());
    }
}
