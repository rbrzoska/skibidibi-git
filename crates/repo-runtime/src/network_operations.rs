use std::{ffi::OsString, path::Path, time::Duration};

use app_domain::{
    AutoStashCreateState, AutoStashOutcome, PopStashRequest, PullOperationState, PullRequest,
    PullResult, PullStrategy, PushAnalysis, PushReadiness, PushRequest, PushResult, PushTarget,
    RepositoryBranchKind, RepositoryStatePrecondition, RepositoryStatus, SetUpstreamRequest,
    SetUpstreamResult, StashCleanupState, StashPushState, StashRestoreState,
};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{
    NavigationGitExecutor, NavigationRuntimeError, RepositoryRuntime, RepositoryRuntimeError,
    StashActionError, StashActionGitExecutor, repository_status,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);
const OUTPUT_LIMIT: usize = 512 * 1024;
const BINDING_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(upstream:short)%00%(upstream:remotename)%00%(upstream:remoteref)%00";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamBinding {
    branch_ref: String,
    head: String,
    upstream: Option<String>,
    remote: Option<String>,
    remote_ref: Option<String>,
}

pub trait NetworkGitExecutor: Send + Sync {
    fn query_upstream_binding(
        &self,
        repository: &Path,
        branch_ref: &str,
    ) -> Result<GitOutput, GitRunError>;
    fn query_remotes(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn fetch_remote(&self, repository: &Path, remote: &str) -> Result<GitOutput, GitRunError>;
    fn merge_upstream(
        &self,
        repository: &Path,
        upstream: &str,
        ff_only: bool,
    ) -> Result<GitOutput, GitRunError>;
    fn rebase_upstream(&self, repository: &Path, upstream: &str) -> Result<GitOutput, GitRunError>;
    fn push_ref(
        &self,
        repository: &Path,
        remote: &str,
        remote_ref: &str,
        set_upstream: bool,
    ) -> Result<GitOutput, GitRunError>;
    fn set_upstream(
        &self,
        repository: &Path,
        branch: &str,
        upstream_full_name: &str,
    ) -> Result<GitOutput, GitRunError>;
}

impl NetworkGitExecutor for GitRunner {
    fn query_upstream_binding(
        &self,
        repository: &Path,
        branch_ref: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["for-each-ref", BINDING_FORMAT, branch_ref],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn query_remotes(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, ["remote"])
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn fetch_remote(&self, repository: &Path, remote: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Network,
                [
                    "-c",
                    "credential.interactive=never",
                    "fetch",
                    "--prune",
                    "--no-write-fetch-head",
                    remote,
                ],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(NETWORK_TIMEOUT),
        )
    }

    fn merge_upstream(
        &self,
        repository: &Path,
        upstream: &str,
        ff_only: bool,
    ) -> Result<GitOutput, GitRunError> {
        let mut args = vec![OsString::from("merge")];
        args.push(OsString::from(if ff_only { "--ff-only" } else { "--ff" }));
        args.push(OsString::from("--no-edit"));
        args.push(OsString::from("--"));
        args.push(OsString::from(upstream));
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, args)
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(NETWORK_TIMEOUT),
        )
    }

    fn rebase_upstream(&self, repository: &Path, upstream: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, ["rebase", "--", upstream])
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(NETWORK_TIMEOUT),
        )
    }

    fn push_ref(
        &self,
        repository: &Path,
        remote: &str,
        remote_ref: &str,
        set_upstream: bool,
    ) -> Result<GitOutput, GitRunError> {
        let mut args = vec![
            OsString::from("-c"),
            OsString::from("credential.interactive=never"),
            OsString::from("push"),
        ];
        if set_upstream {
            args.push(OsString::from("--set-upstream"));
        }
        args.push(OsString::from(remote));
        args.push(OsString::from(format!("HEAD:{remote_ref}")));
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Network, args)
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(NETWORK_TIMEOUT),
        )
    }

    fn set_upstream(
        &self,
        repository: &Path,
        branch: &str,
        upstream_full_name: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    "branch",
                    &format!("--set-upstream-to={upstream_full_name}"),
                    "--",
                    branch,
                ],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum NetworkOperationError {
    #[error("the network operation request is invalid")]
    InvalidRequest,
    #[error("the repository state changed; refresh before retrying")]
    StaleState,
    #[error("the current branch has no configured upstream")]
    NoUpstream,
    #[error("detached or unborn HEAD cannot use this network operation")]
    UnsupportedHead,
    #[error("resolve repository conflicts before this network operation")]
    ConflictsPresent,
    #[error("the working tree is dirty; enable auto-stash to continue")]
    DirtyWorkingTree,
    #[error("the configured upstream changed during the operation")]
    UpstreamChanged,
    #[error("push is blocked because the branch is behind or diverged")]
    PushBlocked,
    #[error("the requested remote or remote branch is unavailable")]
    RemoteUnavailable,
    #[error(transparent)]
    Stash(#[from] StashActionError),
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Navigation(#[from] NavigationRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error("Git returned malformed upstream metadata")]
    InvalidOutput,
}

impl NetworkOperationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::InvalidOutput => "invalidRequest",
            Self::StaleState | Self::UpstreamChanged => "staleState",
            Self::NoUpstream => "noUpstream",
            Self::UnsupportedHead => "unsupportedHead",
            Self::ConflictsPresent => "conflictsPresent",
            Self::DirtyWorkingTree => "dirtyWorkingTree",
            Self::PushBlocked => "pushBlocked",
            Self::RemoteUnavailable => "remoteUnavailable",
            Self::Stash(error) => error.code(),
            Self::Repository(_) | Self::Navigation(_) | Self::Git(_) => "gitRejected",
        }
    }
}

impl<E> RepositoryRuntime<E>
where
    E: NetworkGitExecutor + StashActionGitExecutor + NavigationGitExecutor,
{
    pub fn push_analysis(&self, repository: &Path) -> Result<PushAnalysis, NetworkOperationError> {
        let status = repository_status(&self.executor, repository)?;
        analysis_from_status(&self.executor, repository, &status)
    }

    pub fn pull_repository(
        &self,
        repository: &Path,
        request: &PullRequest,
    ) -> Result<PullResult, NetworkOperationError> {
        let before = repository_status(&self.executor, repository)?;
        validate_precondition(&before, &request.precondition)?;
        validate_network_head(&before)?;
        reject_conflicts(&before)?;
        let head_before = before
            .branch
            .oid
            .clone()
            .ok_or(NetworkOperationError::UnsupportedHead)?;
        let branch = before
            .branch
            .head
            .as_deref()
            .ok_or(NetworkOperationError::UnsupportedHead)?;
        let binding = binding_for(&self.executor, repository, branch)?;
        let remote = binding
            .remote
            .clone()
            .ok_or(NetworkOperationError::NoUpstream)?;
        let upstream = binding
            .upstream
            .clone()
            .ok_or(NetworkOperationError::NoUpstream)?;
        if binding.head != head_before || before.branch.upstream.as_deref() != Some(&upstream) {
            return Err(NetworkOperationError::StaleState);
        }

        let mut auto_stash = AutoStashOutcome::not_requested();
        if !before.entries.is_empty() {
            let options = request
                .auto_stash
                .as_ref()
                .ok_or(NetworkOperationError::DirtyWorkingTree)?;
            let pushed = self.push_stash(
                repository,
                &app_domain::PushStashRequest {
                    message: options.message.clone(),
                    include_untracked: true,
                    precondition: request.precondition.clone(),
                },
            )?;
            auto_stash = auto_stash_from_push(&pushed);
            if pushed.state != StashPushState::Created && pushed.state != StashPushState::NoChanges
            {
                return Ok(PullResult {
                    state: PullOperationState::Failed,
                    head_before,
                    head_after: pushed
                        .status
                        .as_ref()
                        .and_then(|status| status.branch.oid.clone()),
                    status: pushed.status,
                    auto_stash,
                    error_message: pushed
                        .error_message
                        .or_else(|| Some("auto-stash creation was incomplete".to_owned())),
                });
            }
        } else if request.auto_stash.is_some() {
            auto_stash.create = AutoStashCreateState::NotNeeded;
        }

        if let Err(error) = self.executor.fetch_remote(repository, &remote) {
            let status =
                restore_after_safe_failure(self, repository, &head_before, &mut auto_stash);
            return Ok(PullResult {
                state: PullOperationState::Failed,
                head_before,
                head_after: status.as_ref().and_then(|status| status.branch.oid.clone()),
                status,
                auto_stash,
                error_message: Some(error.to_string()),
            });
        }

        let rebound = binding_for(&self.executor, repository, branch)?;
        if rebound.branch_ref != binding.branch_ref
            || rebound.head != head_before
            || rebound.upstream != binding.upstream
            || rebound.remote != binding.remote
            || rebound.remote_ref != binding.remote_ref
        {
            let status =
                restore_after_safe_failure(self, repository, &head_before, &mut auto_stash);
            return Ok(PullResult {
                state: PullOperationState::Failed,
                head_before,
                head_after: status.as_ref().and_then(|status| status.branch.oid.clone()),
                status,
                auto_stash,
                error_message: Some(NetworkOperationError::UpstreamChanged.to_string()),
            });
        }

        let operation = match request.strategy {
            PullStrategy::FfIfPossible => {
                self.executor.merge_upstream(repository, &upstream, false)
            }
            PullStrategy::FfOnly => self.executor.merge_upstream(repository, &upstream, true),
            PullStrategy::Rebase => self.executor.rebase_upstream(repository, &upstream),
        };
        let after_operation = repository_status(&self.executor, repository).ok();
        let conflicted = after_operation.as_ref().is_some_and(has_conflicts);
        if conflicted {
            mark_stash_retained(&mut auto_stash, StashRestoreState::SkippedUnsafe);
            return Ok(PullResult {
                state: PullOperationState::Conflicted,
                head_before,
                head_after: after_operation
                    .as_ref()
                    .and_then(|status| status.branch.oid.clone()),
                status: after_operation,
                auto_stash,
                error_message: operation.err().map(|error| error.to_string()),
            });
        }
        if let Err(error) = operation {
            let status =
                restore_after_safe_failure(self, repository, &head_before, &mut auto_stash);
            return Ok(PullResult {
                state: PullOperationState::Failed,
                head_before,
                head_after: status.as_ref().and_then(|status| status.branch.oid.clone()),
                status,
                auto_stash,
                error_message: Some(error.to_string()),
            });
        }

        let status = restore_after_success(self, repository, after_operation, &mut auto_stash);
        let state = if status.as_ref().is_some_and(has_conflicts) {
            PullOperationState::Conflicted
        } else if auto_stash_incomplete(&auto_stash) {
            PullOperationState::Failed
        } else {
            PullOperationState::Succeeded
        };
        let error_message =
            (state == PullOperationState::Failed).then(|| auto_stash_failure_message(&auto_stash));
        Ok(PullResult {
            state,
            head_before,
            head_after: status.as_ref().and_then(|status| status.branch.oid.clone()),
            status,
            auto_stash,
            error_message,
        })
    }

    pub fn push_repository(
        &self,
        repository: &Path,
        request: &PushRequest,
    ) -> Result<PushResult, NetworkOperationError> {
        let before = repository_status(&self.executor, repository)?;
        validate_precondition(&before, &request.precondition)?;
        validate_network_head(&before)?;
        reject_conflicts(&before)?;
        let branch = before
            .branch
            .head
            .as_deref()
            .ok_or(NetworkOperationError::UnsupportedHead)?;

        let (remote, remote_ref, set_upstream) = match &request.target {
            PushTarget::Configured { expected_upstream } => {
                let binding = binding_for(&self.executor, repository, branch)?;
                if binding.upstream.as_deref() != Some(expected_upstream) {
                    return Err(NetworkOperationError::UpstreamChanged);
                }
                (
                    binding.remote.ok_or(NetworkOperationError::NoUpstream)?,
                    binding
                        .remote_ref
                        .ok_or(NetworkOperationError::NoUpstream)?,
                    false,
                )
            }
            PushTarget::SetUpstream {
                remote,
                remote_branch,
            } => {
                validate_remote_name(remote)?;
                validate_remote_branch(remote_branch)?;
                ensure_remote_exists(&self.executor, repository, remote)?;
                (remote.clone(), format!("refs/heads/{remote_branch}"), true)
            }
        };

        self.executor.fetch_remote(repository, &remote)?;
        let refreshed = repository_status(&self.executor, repository)?;
        if refreshed.branch.oid != before.branch.oid || refreshed.branch.head != before.branch.head
        {
            return Err(NetworkOperationError::StaleState);
        }
        reject_conflicts(&refreshed)?;
        if !set_upstream {
            let analysis = analysis_from_status(&self.executor, repository, &refreshed)?;
            if matches!(
                analysis.readiness,
                PushReadiness::Behind | PushReadiness::Diverged
            ) {
                return Err(NetworkOperationError::PushBlocked);
            }
            if analysis.readiness == PushReadiness::UpToDate {
                return Ok(PushResult {
                    pushed: false,
                    analysis,
                    status: refreshed,
                });
            }
        }

        self.executor
            .push_ref(repository, &remote, &remote_ref, set_upstream)?;
        let status = repository_status(&self.executor, repository)?;
        let analysis = analysis_from_status(&self.executor, repository, &status)?;
        Ok(PushResult {
            pushed: true,
            analysis,
            status,
        })
    }

    pub fn set_repository_upstream(
        &self,
        repository: &Path,
        request: &SetUpstreamRequest,
    ) -> Result<SetUpstreamResult, NetworkOperationError> {
        let status = repository_status(&self.executor, repository)?;
        validate_precondition(&status, &request.precondition)?;
        validate_network_head(&status)?;
        let branch = self
            .branches(repository)?
            .into_iter()
            .find(|branch| branch.full_name == request.remote_full_name)
            .filter(|branch| {
                branch.kind == RepositoryBranchKind::Remote
                    && branch.symbolic_target.is_none()
                    && branch.oid == request.expected_oid
            })
            .ok_or(NetworkOperationError::RemoteUnavailable)?;
        let local = status
            .branch
            .head
            .as_deref()
            .ok_or(NetworkOperationError::UnsupportedHead)?;
        self.executor
            .set_upstream(repository, local, &branch.full_name)?;
        let status = repository_status(&self.executor, repository)?;
        Ok(SetUpstreamResult {
            upstream: branch.name,
            status,
        })
    }
}

fn binding_for<E: NetworkGitExecutor>(
    executor: &E,
    repository: &Path,
    branch: &str,
) -> Result<UpstreamBinding, NetworkOperationError> {
    if !valid_local_branch(branch) {
        return Err(NetworkOperationError::InvalidRequest);
    }
    let full_ref = format!("refs/heads/{branch}");
    let output = executor.query_upstream_binding(repository, &full_ref)?;
    parse_binding(&output.stdout, &full_ref)
}

fn parse_binding(
    input: &[u8],
    expected_ref: &str,
) -> Result<UpstreamBinding, NetworkOperationError> {
    let input = input.strip_suffix(b"\n").unwrap_or(input);
    let fields = input.split(|byte| *byte == 0).collect::<Vec<_>>();
    let [branch_ref, head, upstream, remote, remote_ref, trailing] = fields.as_slice() else {
        return Err(NetworkOperationError::InvalidOutput);
    };
    if !trailing.is_empty() {
        return Err(NetworkOperationError::InvalidOutput);
    }
    let branch_ref = decode_field(branch_ref)?;
    let head = decode_field(head)?;
    if branch_ref != expected_ref || !valid_oid(head) {
        return Err(NetworkOperationError::InvalidOutput);
    }
    let optional = |value: &[u8]| -> Result<Option<String>, NetworkOperationError> {
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(decode_field(value)?.to_owned()))
        }
    };
    let binding = UpstreamBinding {
        branch_ref: branch_ref.to_owned(),
        head: head.to_owned(),
        upstream: optional(upstream)?,
        remote: optional(remote)?,
        remote_ref: optional(remote_ref)?,
    };
    if binding.upstream.is_some() != binding.remote.is_some()
        || binding.upstream.is_some() != binding.remote_ref.is_some()
        || binding
            .remote
            .as_deref()
            .is_some_and(|remote| validate_remote_name(remote).is_err())
        || binding
            .remote_ref
            .as_deref()
            .is_some_and(|value| !valid_remote_ref(value))
    {
        return Err(NetworkOperationError::InvalidOutput);
    }
    Ok(binding)
}

fn decode_field(value: &[u8]) -> Result<&str, NetworkOperationError> {
    std::str::from_utf8(value).map_err(|_| NetworkOperationError::InvalidOutput)
}

fn analysis_from_status<E: NetworkGitExecutor>(
    executor: &E,
    repository: &Path,
    status: &RepositoryStatus,
) -> Result<PushAnalysis, NetworkOperationError> {
    validate_network_head(status)?;
    let branch = status
        .branch
        .head
        .clone()
        .ok_or(NetworkOperationError::UnsupportedHead)?;
    let head = status
        .branch
        .oid
        .clone()
        .ok_or(NetworkOperationError::UnsupportedHead)?;
    let binding = binding_for(executor, repository, &branch)?;
    if binding.head != head || binding.upstream != status.branch.upstream {
        return Err(NetworkOperationError::StaleState);
    }
    let readiness = match (
        binding.upstream.is_some(),
        status.branch.ahead,
        status.branch.behind,
    ) {
        (false, _, _) => PushReadiness::NoUpstream,
        (true, 0, 0) => PushReadiness::UpToDate,
        (true, _, 0) => PushReadiness::Ready,
        (true, 0, _) => PushReadiness::Behind,
        (true, _, _) => PushReadiness::Diverged,
    };
    Ok(PushAnalysis {
        branch,
        head,
        upstream: binding.upstream,
        remote: binding.remote,
        remote_ref: binding.remote_ref,
        ahead: status.branch.ahead,
        behind: status.branch.behind,
        readiness,
    })
}

fn validate_precondition(
    status: &RepositoryStatus,
    expected: &RepositoryStatePrecondition,
) -> Result<(), NetworkOperationError> {
    if expected.expected_index_fingerprint.is_empty()
        || expected.expected_worktree_fingerprint.is_empty()
        || status.branch.oid != expected.expected_head
        || status.branch.head != expected.expected_head_name
        || status.branch.detached != expected.expected_detached
        || status.branch.unborn != expected.expected_unborn
        || status.index_fingerprint != expected.expected_index_fingerprint
        || status.worktree_fingerprint != expected.expected_worktree_fingerprint
    {
        return Err(NetworkOperationError::StaleState);
    }
    Ok(())
}

fn validate_network_head(status: &RepositoryStatus) -> Result<(), NetworkOperationError> {
    if status.branch.detached
        || status.branch.unborn
        || status.branch.oid.is_none()
        || status.branch.head.is_none()
    {
        return Err(NetworkOperationError::UnsupportedHead);
    }
    Ok(())
}

fn reject_conflicts(status: &RepositoryStatus) -> Result<(), NetworkOperationError> {
    if has_conflicts(status) {
        Err(NetworkOperationError::ConflictsPresent)
    } else {
        Ok(())
    }
}

fn has_conflicts(status: &RepositoryStatus) -> bool {
    status
        .entries
        .iter()
        .any(|entry| entry.kind == app_domain::StatusEntryKind::Unmerged)
}

fn auto_stash_from_push(push: &app_domain::PushStashResult) -> AutoStashOutcome {
    AutoStashOutcome {
        create: match push.state {
            StashPushState::NoChanges => AutoStashCreateState::NotNeeded,
            StashPushState::Created => AutoStashCreateState::Created,
            StashPushState::Failed => AutoStashCreateState::Failed,
            StashPushState::Partial => AutoStashCreateState::Partial,
        },
        stash: push.stash.clone(),
        restore: StashRestoreState::NotRequired,
        cleanup: if push.stash.is_some() {
            StashCleanupState::Retained
        } else {
            StashCleanupState::NotRequired
        },
        create_error: push.error_message.clone(),
        restore_error: None,
        cleanup_error: None,
    }
}

fn restore_after_success<E>(
    runtime: &RepositoryRuntime<E>,
    repository: &Path,
    status: Option<RepositoryStatus>,
    outcome: &mut AutoStashOutcome,
) -> Option<RepositoryStatus>
where
    E: NetworkGitExecutor + StashActionGitExecutor + NavigationGitExecutor,
{
    let Some(stash) = outcome.stash.clone() else {
        return status;
    };
    let Some(status) = status else {
        mark_stash_retained(outcome, StashRestoreState::Failed);
        outcome.restore_error = Some("post-pull repository status could not be loaded".to_owned());
        return None;
    };
    match runtime.pop_stash(
        repository,
        &PopStashRequest {
            stash,
            restore_index: true,
            precondition: precondition_from_status(&status),
        },
    ) {
        Ok(result) => {
            outcome.restore = result.restore;
            outcome.cleanup = result.cleanup;
            outcome.restore_error = result.restore_error;
            outcome.cleanup_error = result.cleanup_error;
            result.status
        }
        Err(error) => {
            mark_stash_retained(outcome, StashRestoreState::Failed);
            outcome.restore_error = Some(error.to_string());
            Some(status)
        }
    }
}

fn restore_after_safe_failure<E>(
    runtime: &RepositoryRuntime<E>,
    repository: &Path,
    expected_head: &str,
    outcome: &mut AutoStashOutcome,
) -> Option<RepositoryStatus>
where
    E: NetworkGitExecutor + StashActionGitExecutor + NavigationGitExecutor,
{
    let status = repository_status(&runtime.executor, repository).ok();
    let safe = status.as_ref().is_some_and(|status| {
        status.branch.oid.as_deref() == Some(expected_head)
            && status.entries.is_empty()
            && !has_conflicts(status)
    });
    if safe {
        restore_after_success(runtime, repository, status, outcome)
    } else {
        mark_stash_retained(outcome, StashRestoreState::SkippedUnsafe);
        status
    }
}

fn mark_stash_retained(outcome: &mut AutoStashOutcome, restore: StashRestoreState) {
    if outcome.stash.is_some() {
        outcome.restore = restore;
        outcome.cleanup = StashCleanupState::Retained;
    }
}

fn auto_stash_incomplete(outcome: &AutoStashOutcome) -> bool {
    matches!(
        outcome.create,
        AutoStashCreateState::Failed | AutoStashCreateState::Partial
    ) || matches!(
        outcome.restore,
        StashRestoreState::Failed | StashRestoreState::SkippedUnsafe
    ) || matches!(
        outcome.cleanup,
        StashCleanupState::Retained | StashCleanupState::Failed
    )
}

fn auto_stash_failure_message(outcome: &AutoStashOutcome) -> String {
    outcome
        .restore_error
        .as_deref()
        .or(outcome.cleanup_error.as_deref())
        .or(outcome.create_error.as_deref())
        .unwrap_or("pull completed, but auto-stash recovery requires attention")
        .to_owned()
}

fn precondition_from_status(status: &RepositoryStatus) -> RepositoryStatePrecondition {
    RepositoryStatePrecondition {
        expected_head: status.branch.oid.clone(),
        expected_head_name: status.branch.head.clone(),
        expected_detached: status.branch.detached,
        expected_unborn: status.branch.unborn,
        expected_index_fingerprint: status.index_fingerprint.clone(),
        expected_worktree_fingerprint: status.worktree_fingerprint.clone(),
    }
}

fn ensure_remote_exists<E: NetworkGitExecutor>(
    executor: &E,
    repository: &Path,
    remote: &str,
) -> Result<(), NetworkOperationError> {
    let output = executor.query_remotes(repository)?;
    let found = output
        .stdout
        .split(|byte| *byte == b'\n')
        .any(|line| line == remote.as_bytes());
    if found {
        Ok(())
    } else {
        Err(NetworkOperationError::RemoteUnavailable)
    }
}

fn validate_remote_name(value: &str) -> Result<(), NetworkOperationError> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        Err(NetworkOperationError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn validate_remote_branch(value: &str) -> Result<(), NetworkOperationError> {
    if valid_local_branch(value) {
        Ok(())
    } else {
        Err(NetworkOperationError::InvalidRequest)
    }
}

fn valid_local_branch(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.ends_with('.')
        && !value.contains("..")
        && !value.contains("@{")
        && !value.contains("//")
        && !value
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
        && !value.bytes().any(|byte| {
            matches!(
                byte,
                0x00..=0x20 | 0x7f | b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\'
            )
        })
}

fn valid_remote_ref(value: &str) -> bool {
    value
        .strip_prefix("refs/heads/")
        .is_some_and(valid_local_branch)
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "0123456789012345678901234567890123456789";

    #[test]
    fn parses_exact_upstream_binding_and_rejects_mismatch() {
        let input = format!("refs/heads/main\0{OID}\0origin/main\0origin\0refs/heads/main\0\n");
        let binding = parse_binding(input.as_bytes(), "refs/heads/main").unwrap();
        assert_eq!(binding.remote.as_deref(), Some("origin"));
        assert_eq!(binding.remote_ref.as_deref(), Some("refs/heads/main"));
        assert!(parse_binding(input.as_bytes(), "refs/heads/other").is_err());
    }

    #[test]
    fn push_readiness_distinguishes_all_sync_states() {
        let readiness = |upstream: bool, ahead, behind| match (upstream, ahead, behind) {
            (false, _, _) => PushReadiness::NoUpstream,
            (true, 0, 0) => PushReadiness::UpToDate,
            (true, _, 0) => PushReadiness::Ready,
            (true, 0, _) => PushReadiness::Behind,
            (true, _, _) => PushReadiness::Diverged,
        };
        assert_eq!(readiness(false, 0, 0), PushReadiness::NoUpstream);
        assert_eq!(readiness(true, 0, 0), PushReadiness::UpToDate);
        assert_eq!(readiness(true, 2, 0), PushReadiness::Ready);
        assert_eq!(readiness(true, 0, 2), PushReadiness::Behind);
        assert_eq!(readiness(true, 2, 2), PushReadiness::Diverged);
    }
}
