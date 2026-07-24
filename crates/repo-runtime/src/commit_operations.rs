use std::{path::Path, time::Duration};

use app_domain::{
    CommitOperationRequest, CommitOperationResult, CommitOperationState,
    RepositoryStatePrecondition, RepositoryStatus, ResetCommitRequest, ResetMode, StatusEntryKind,
};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{
    RepositoryRuntime, RepositoryRuntimeError, RepositoryStatusGitExecutor, repository_status,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(120);
const OUTPUT_LIMIT: usize = 512 * 1024;
const ERROR_MESSAGE_LIMIT: usize = 4096;
const COMMIT_IDENTITY_FORMAT: &str = "--format=%H%x00%P";

pub trait CommitOperationGitExecutor: RepositoryStatusGitExecutor {
    fn inspect_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
    fn cherry_pick_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
    fn revert_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
    fn reset_commit(
        &self,
        repository: &Path,
        oid: &str,
        mode: ResetMode,
    ) -> Result<GitOutput, GitRunError>;
    fn current_head_oid(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
}

impl CommitOperationGitExecutor for GitRunner {
    fn inspect_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["show", "-s", COMMIT_IDENTITY_FORMAT, oid, "--"],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn cherry_pick_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                commit_action_arguments("cherry-pick", oid),
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn revert_commit(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                commit_action_arguments("revert", oid),
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn reset_commit(
        &self,
        repository: &Path,
        oid: &str,
        mode: ResetMode,
    ) -> Result<GitOutput, GitRunError> {
        let mode = match mode {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        };
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, ["reset", mode, oid, "--"])
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn current_head_oid(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["rev-parse", "--verify", "--quiet", "HEAD"],
            )
            .with_output_limits(256, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum CommitOperationError {
    #[error("the commit operation request is invalid")]
    InvalidRequest,
    #[error("repository state changed; refresh before retrying")]
    StaleState,
    #[error("commit operations require an existing local branch")]
    UnsupportedHead,
    #[error("cherry-pick and revert require a clean index and working tree")]
    DirtyWorkingTree,
    #[error("merge commits are not supported by this operation")]
    MergeCommitUnsupported,
    #[error("hard reset requires explicit destructive confirmation")]
    HardResetConfirmationRequired,
    #[error("Git returned malformed commit identity data")]
    InvalidCommitIdentity,
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
}

impl CommitOperationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::InvalidCommitIdentity => "invalidRequest",
            Self::StaleState => "staleState",
            Self::UnsupportedHead => "unsupportedHead",
            Self::DirtyWorkingTree => "dirtyWorkingTree",
            Self::MergeCommitUnsupported => "mergeCommitUnsupported",
            Self::HardResetConfirmationRequired => "hardResetConfirmationRequired",
            Self::Repository(_) => "internal",
            Self::Git(GitRunError::TimedOut { .. }) => "timedOut",
            Self::Git(GitRunError::OutputLimitExceeded { .. }) => "outputLimit",
            Self::Git(_) => "gitRejected",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::StaleState | Self::Git(GitRunError::TimedOut { .. })
        )
    }
}

#[derive(Debug, Clone, Copy)]
enum CommitAction {
    CherryPick,
    Revert,
}

impl<E: CommitOperationGitExecutor> RepositoryRuntime<E> {
    pub fn cherry_pick_commit(
        &self,
        repository: &Path,
        request: &CommitOperationRequest,
    ) -> Result<CommitOperationResult, CommitOperationError> {
        self.apply_commit(repository, request, CommitAction::CherryPick)
    }

    pub fn revert_commit(
        &self,
        repository: &Path,
        request: &CommitOperationRequest,
    ) -> Result<CommitOperationResult, CommitOperationError> {
        self.apply_commit(repository, request, CommitAction::Revert)
    }

    pub fn reset_commit(
        &self,
        repository: &Path,
        request: &ResetCommitRequest,
    ) -> Result<CommitOperationResult, CommitOperationError> {
        validate_oid(&request.target_oid)?;
        validate_precondition_shape(&request.precondition)?;
        if request.mode == ResetMode::Hard && !request.confirm_hard_reset {
            return Err(CommitOperationError::HardResetConfirmationRequired);
        }

        let before = repository_status(&self.executor, repository)?;
        validate_precondition(&before, &request.precondition)?;
        validate_local_head(&before)?;
        let head_before = before
            .branch
            .oid
            .clone()
            .ok_or(CommitOperationError::UnsupportedHead)?;
        let target = inspect_commit(&self.executor, repository, &request.target_oid)?;

        let immediately_before = repository_status(&self.executor, repository)?;
        validate_precondition(&immediately_before, &request.precondition)?;
        validate_local_head(&immediately_before)?;

        let mutation = self
            .executor
            .reset_commit(repository, &target.oid, request.mode);
        self.finish_reset(
            repository,
            mutation,
            target.oid,
            head_before,
            &immediately_before,
        )
    }

    fn apply_commit(
        &self,
        repository: &Path,
        request: &CommitOperationRequest,
        action: CommitAction,
    ) -> Result<CommitOperationResult, CommitOperationError> {
        validate_oid(&request.target_oid)?;
        validate_precondition_shape(&request.precondition)?;
        let before = repository_status(&self.executor, repository)?;
        validate_precondition(&before, &request.precondition)?;
        validate_local_head(&before)?;
        require_clean(&before)?;
        let head_before = before
            .branch
            .oid
            .clone()
            .ok_or(CommitOperationError::UnsupportedHead)?;
        let target = inspect_non_merge_commit(&self.executor, repository, &request.target_oid)?;

        let immediately_before = repository_status(&self.executor, repository)?;
        validate_precondition(&immediately_before, &request.precondition)?;
        validate_local_head(&immediately_before)?;
        require_clean(&immediately_before)?;

        let mutation = match action {
            CommitAction::CherryPick => self.executor.cherry_pick_commit(repository, &target.oid),
            CommitAction::Revert => self.executor.revert_commit(repository, &target.oid),
        };
        self.finish_commit(
            repository,
            mutation,
            target.oid,
            head_before,
            &immediately_before,
        )
    }

    fn finish_commit(
        &self,
        repository: &Path,
        mutation: Result<GitOutput, GitRunError>,
        target_oid: String,
        head_before: String,
        before: &RepositoryStatus,
    ) -> Result<CommitOperationResult, CommitOperationError> {
        let status = match repository_status(&self.executor, repository) {
            Ok(status) => status,
            Err(error) => {
                return Ok(unknown_result(
                    &self.executor,
                    repository,
                    target_oid,
                    head_before,
                    None,
                    format!("the operation outcome could not be inspected: {error}"),
                ));
            }
        };
        let head_after = status.branch.oid.clone();

        if let Err(error) = mutation {
            if has_conflicts(&status) {
                return Ok(CommitOperationResult {
                    state: CommitOperationState::Conflicted,
                    target_oid,
                    head_before,
                    head_after,
                    status: Some(status),
                    error_message: Some(bounded_error(&error)),
                    mutation_may_have_occurred: true,
                });
            }
            return Ok(unknown_result(
                &self.executor,
                repository,
                target_oid,
                head_before,
                Some(status),
                format!(
                    "Git did not complete the operation: {}",
                    bounded_error(&error)
                ),
            ));
        }

        let Some(new_oid) = head_after.clone() else {
            return Ok(unknown_result(
                &self.executor,
                repository,
                target_oid,
                head_before,
                Some(status),
                "Git reported success but HEAD has no commit".to_owned(),
            ));
        };
        let same_branch = status.branch.head == before.branch.head
            && !status.branch.detached
            && !status.branch.unborn;
        let parent_matches = inspect_commit(&self.executor, repository, &new_oid)
            .map(|commit| commit.parents.as_slice() == [head_before.as_str()])
            .unwrap_or(false);
        if !same_branch || new_oid == head_before || !parent_matches {
            return Ok(unknown_result(
                &self.executor,
                repository,
                target_oid,
                head_before,
                Some(status),
                "the resulting commit did not satisfy the expected postcondition".to_owned(),
            ));
        }

        Ok(CommitOperationResult {
            state: CommitOperationState::Succeeded,
            target_oid,
            head_before,
            head_after: Some(new_oid),
            status: Some(status),
            error_message: None,
            mutation_may_have_occurred: true,
        })
    }

    fn finish_reset(
        &self,
        repository: &Path,
        mutation: Result<GitOutput, GitRunError>,
        target_oid: String,
        head_before: String,
        before: &RepositoryStatus,
    ) -> Result<CommitOperationResult, CommitOperationError> {
        let status = match repository_status(&self.executor, repository) {
            Ok(status) => status,
            Err(error) => {
                return Ok(unknown_result(
                    &self.executor,
                    repository,
                    target_oid,
                    head_before,
                    None,
                    format!("the reset outcome could not be inspected: {error}"),
                ));
            }
        };
        if let Err(error) = mutation {
            return Ok(unknown_result(
                &self.executor,
                repository,
                target_oid,
                head_before,
                Some(status),
                format!("Git did not complete the reset: {}", bounded_error(&error)),
            ));
        }
        let head_after = status.branch.oid.clone();
        let verified = status.branch.head == before.branch.head
            && !status.branch.detached
            && !status.branch.unborn
            && head_after.as_deref() == Some(target_oid.as_str());
        if !verified {
            return Ok(unknown_result(
                &self.executor,
                repository,
                target_oid,
                head_before,
                Some(status),
                "the reset HEAD did not satisfy the expected postcondition".to_owned(),
            ));
        }

        Ok(CommitOperationResult {
            state: CommitOperationState::Succeeded,
            target_oid,
            head_before,
            head_after,
            status: Some(status),
            error_message: None,
            mutation_may_have_occurred: true,
        })
    }
}

#[derive(Debug)]
struct CommitIdentity {
    oid: String,
    parents: Vec<String>,
}

fn inspect_non_merge_commit<E: CommitOperationGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
) -> Result<CommitIdentity, CommitOperationError> {
    let identity = inspect_commit(executor, repository, oid)?;
    if identity.parents.len() > 1 {
        return Err(CommitOperationError::MergeCommitUnsupported);
    }
    Ok(identity)
}

fn commit_action_arguments<'a>(command: &'a str, oid: &'a str) -> [&'a str; 3] {
    [command, "--no-edit", oid]
}

fn inspect_commit<E: CommitOperationGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
) -> Result<CommitIdentity, CommitOperationError> {
    validate_oid(oid)?;
    let output = executor.inspect_commit(repository, oid)?;
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| CommitOperationError::InvalidCommitIdentity)?
        .trim_end_matches(['\r', '\n']);
    let (resolved, parents) = text
        .split_once('\0')
        .ok_or(CommitOperationError::InvalidCommitIdentity)?;
    validate_oid(resolved)?;
    if !resolved.eq_ignore_ascii_case(oid) {
        return Err(CommitOperationError::InvalidCommitIdentity);
    }
    let parents = parents
        .split_ascii_whitespace()
        .map(|parent| {
            validate_oid(parent)?;
            Ok(parent.to_ascii_lowercase())
        })
        .collect::<Result<Vec<_>, CommitOperationError>>()?;
    Ok(CommitIdentity {
        oid: resolved.to_ascii_lowercase(),
        parents,
    })
}

fn validate_oid(oid: &str) -> Result<(), CommitOperationError> {
    if matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(CommitOperationError::InvalidRequest)
    }
}

fn validate_precondition_shape(
    precondition: &RepositoryStatePrecondition,
) -> Result<(), CommitOperationError> {
    if precondition.expected_index_fingerprint.is_empty()
        || precondition.expected_worktree_fingerprint.is_empty()
        || precondition
            .expected_head
            .as_deref()
            .is_none_or(|oid| validate_oid(oid).is_err())
        || precondition
            .expected_head_name
            .as_deref()
            .is_none_or(str::is_empty)
        || precondition.expected_detached
        || precondition.expected_unborn
    {
        return Err(CommitOperationError::InvalidRequest);
    }
    Ok(())
}

fn validate_precondition(
    status: &RepositoryStatus,
    expected: &RepositoryStatePrecondition,
) -> Result<(), CommitOperationError> {
    if status.branch.oid != expected.expected_head
        || status.branch.head != expected.expected_head_name
        || status.branch.detached != expected.expected_detached
        || status.branch.unborn != expected.expected_unborn
        || status.index_fingerprint != expected.expected_index_fingerprint
        || status.worktree_fingerprint != expected.expected_worktree_fingerprint
    {
        return Err(CommitOperationError::StaleState);
    }
    Ok(())
}

fn validate_local_head(status: &RepositoryStatus) -> Result<(), CommitOperationError> {
    if status.branch.oid.is_none()
        || status.branch.head.as_deref().is_none_or(str::is_empty)
        || status.branch.detached
        || status.branch.unborn
    {
        return Err(CommitOperationError::UnsupportedHead);
    }
    Ok(())
}

fn require_clean(status: &RepositoryStatus) -> Result<(), CommitOperationError> {
    if status.entries.is_empty() {
        Ok(())
    } else {
        Err(CommitOperationError::DirtyWorkingTree)
    }
}

fn has_conflicts(status: &RepositoryStatus) -> bool {
    status
        .entries
        .iter()
        .any(|entry| entry.kind == StatusEntryKind::Unmerged)
}

fn unknown_result<E: CommitOperationGitExecutor>(
    executor: &E,
    repository: &Path,
    target_oid: String,
    head_before: String,
    status: Option<RepositoryStatus>,
    error_message: String,
) -> CommitOperationResult {
    let head_after = status
        .as_ref()
        .and_then(|status| status.branch.oid.clone())
        .or_else(|| exact_head_oid(executor, repository));
    CommitOperationResult {
        state: CommitOperationState::OutcomeUnknown,
        target_oid,
        head_before,
        head_after,
        status,
        error_message: Some(bounded_message(&error_message)),
        mutation_may_have_occurred: true,
    }
}

fn exact_head_oid<E: CommitOperationGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Option<String> {
    let output = executor.current_head_oid(repository).ok()?;
    let oid = std::str::from_utf8(&output.stdout).ok()?.trim();
    validate_oid(oid).ok()?;
    Some(oid.to_ascii_lowercase())
}

fn bounded_error(error: &GitRunError) -> String {
    bounded_message(&error.to_string())
}

fn bounded_message(message: &str) -> String {
    let mut sanitized = message
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\t') {
                '�'
            } else {
                character
            }
        })
        .take(ERROR_MESSAGE_LIMIT)
        .collect::<String>();
    if message.chars().count() > ERROR_MESSAGE_LIMIT {
        sanitized.push('…');
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_action_argv_preserves_repository_signing_policy() {
        let oid = "a".repeat(40);
        let cherry_pick = commit_action_arguments("cherry-pick", &oid);
        let revert = commit_action_arguments("revert", &oid);

        assert_eq!(cherry_pick, ["cherry-pick", "--no-edit", oid.as_str()]);
        assert_eq!(revert, ["revert", "--no-edit", oid.as_str()]);
        assert!(!cherry_pick.contains(&"--no-gpg-sign"));
        assert!(!revert.contains(&"--no-gpg-sign"));
    }

    #[test]
    fn only_full_sha1_or_sha256_object_ids_are_accepted() {
        assert!(validate_oid(&"a".repeat(40)).is_ok());
        assert!(validate_oid(&"B".repeat(64)).is_ok());
        assert!(validate_oid(&"a".repeat(39)).is_err());
        assert!(validate_oid(&"a".repeat(65)).is_err());
        assert!(validate_oid(&format!("{};", "a".repeat(39))).is_err());
    }
}
