use std::{path::Path, time::Duration};

use app_domain::{
    AutoStashCreateState, AutoStashOutcome, PopStashRequest, PushStashRequest, RepositoryBranch,
    RepositoryBranchKind, RepositoryStatePrecondition, RepositoryStatus, StashCleanupState,
    StashIdentity, StashPushState, StashRestoreState, SwitchBranchRequest, SwitchBranchResult,
};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, NavigationParseError,
    parse_branch_records,
};
use thiserror::Error;

use super::{RepositoryRuntime, RepositoryRuntimeError, StashActionError, StashActionGitExecutor};
use crate::repository_status;

const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const BRANCH_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const ACTION_STDOUT_LIMIT: usize = 256 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const BRANCH_FORMAT: &str = "--format=%(refname)%00%(refname:short)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(symref)%00";
const MAX_STASH_MESSAGE_BYTES: usize = 512;

/// Narrow executor boundary that keeps branch discovery read-only and switching explicitly
/// mutating. Implementations must pass argv directly to Git without a shell.
pub trait BranchActionGitExecutor: Send + Sync {
    fn query_switchable_branches(&self, repository: &Path) -> Result<GitOutput, GitRunError>;

    fn switch_existing_local_branch(
        &self,
        repository: &Path,
        branch_name: &str,
    ) -> Result<GitOutput, GitRunError>;
}

impl BranchActionGitExecutor for GitRunner {
    fn query_switchable_branches(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["for-each-ref", BRANCH_FORMAT, "refs/heads", "refs/remotes"],
            )
            .with_output_limits(BRANCH_OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn switch_existing_local_branch(
        &self,
        repository: &Path,
        branch_name: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["switch", "--no-guess", "--", branch_name],
            )
            .with_output_limits(ACTION_STDOUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum BranchSwitchError {
    #[error("branch ref must be a valid fully-qualified local ref under refs/heads")]
    InvalidLocalRef,
    #[error("remote branch refs cannot be checked out by this operation")]
    RemoteBranchNotAllowed,
    #[error("local branch does not exist")]
    BranchNotFound,
    #[error("symbolic branch refs cannot be checked out by this operation")]
    SymbolicBranchNotAllowed,
    #[error("the working tree has uncommitted changes")]
    DirtyWorktree,
    #[error("the stash message is invalid")]
    InvalidStashMessage,
    #[error("the target branch changed since the switch was prepared")]
    TargetChanged,
    #[error(transparent)]
    Stash(#[from] StashActionError),
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    InvalidOutput(#[from] NavigationParseError),
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
}

impl<E> RepositoryRuntime<E>
where
    E: BranchActionGitExecutor + StashActionGitExecutor,
{
    /// Switches only to a freshly-discovered, exact local branch. It never forces checkout,
    /// discards changes, or contacts a remote. Stashing requires an explicit request.
    pub fn switch_branch(
        &self,
        repository: &Path,
        request: &SwitchBranchRequest,
    ) -> Result<SwitchBranchResult, BranchSwitchError> {
        validate_requested_ref(&request.full_name)?;

        let output = self.executor.query_switchable_branches(repository)?;
        let branches = parse_branch_records(&output.stdout)?;
        let branch = branches
            .iter()
            .find(|branch| branch.full_name == request.full_name)
            .ok_or(BranchSwitchError::BranchNotFound)?;

        if branch.kind != RepositoryBranchKind::Local {
            return Err(BranchSwitchError::RemoteBranchNotAllowed);
        }
        if branch.symbolic_target.is_some() {
            return Err(BranchSwitchError::SymbolicBranchNotAllowed);
        }
        if branch.oid != request.expected_oid {
            return Err(BranchSwitchError::TargetChanged);
        }

        let mut result = result_from_branch(branch, !branch.current);
        if branch.current {
            if request.stash_on_dirty {
                result.auto_stash.create = AutoStashCreateState::NotNeeded;
            }
            return Ok(result);
        }

        let status = repository_status(&self.executor, repository)?;
        if !status.entries.is_empty() {
            if !request.stash_on_dirty {
                return Err(BranchSwitchError::DirtyWorktree);
            }
            let message = request
                .stash_message
                .as_deref()
                .filter(|message| valid_stash_message(message))
                .ok_or(BranchSwitchError::InvalidStashMessage)?;
            let push = self.push_stash(
                repository,
                &PushStashRequest {
                    message: message.to_owned(),
                    include_untracked: true,
                    precondition: precondition_from_status(&status),
                },
            )?;
            result.auto_stash = outcome_from_push(&push);
            result.stash_created = push.stash.is_some();
            match push.state {
                StashPushState::Created => {}
                StashPushState::NoChanges => {}
                StashPushState::Failed | StashPushState::Partial => {
                    result.changed = false;
                    result.operation_succeeded = false;
                    result.operation_error = push.error_message.or_else(|| {
                        Some("the working tree could not be safely stashed".to_owned())
                    });
                    return Ok(result);
                }
            }
        } else if request.stash_on_dirty {
            result.auto_stash.create = AutoStashCreateState::NotNeeded;
        }

        let rebound = match rebind_target(self, repository, request) {
            Ok(branch) => branch,
            Err(error) => {
                if let Some(stash) = result.auto_stash.stash.clone() {
                    result.changed = false;
                    result.operation_succeeded = false;
                    result.operation_error = Some(error.to_string());
                    restore_after_failed_switch(
                        self,
                        repository,
                        &status,
                        &stash,
                        &mut result.auto_stash,
                    );
                    return Ok(result);
                }
                return Err(error);
            }
        };
        result.head = rebound.oid.clone();
        let branch_name = rebound
            .full_name
            .strip_prefix("refs/heads/")
            .ok_or(BranchSwitchError::InvalidLocalRef)?;
        if let Err(source) = self
            .executor
            .switch_existing_local_branch(repository, branch_name)
        {
            if let Some(stash) = result.auto_stash.stash.clone() {
                result.changed = false;
                result.operation_succeeded = false;
                result.operation_error = Some(source.to_string());
                restore_after_failed_switch(
                    self,
                    repository,
                    &status,
                    &stash,
                    &mut result.auto_stash,
                );
                return Ok(result);
            }
            return Err(BranchSwitchError::Git(source));
        }

        if let Some(stash) = result.auto_stash.stash.clone() {
            restore_after_successful_switch(self, repository, &stash, &mut result.auto_stash);
        }
        Ok(result)
    }
}

impl BranchSwitchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DirtyWorktree => "dirtyWorkingTree",
            Self::InvalidLocalRef | Self::InvalidStashMessage => "invalidRequest",
            Self::TargetChanged => "targetChanged",
            Self::RemoteBranchNotAllowed
            | Self::BranchNotFound
            | Self::SymbolicBranchNotAllowed => "branchUnavailable",
            Self::Stash(error) => error.code(),
            Self::Git(_) | Self::InvalidOutput(_) | Self::Repository(_) => "gitRejected",
        }
    }
}

fn rebind_target<E>(
    runtime: &RepositoryRuntime<E>,
    repository: &Path,
    request: &SwitchBranchRequest,
) -> Result<RepositoryBranch, BranchSwitchError>
where
    E: BranchActionGitExecutor + StashActionGitExecutor,
{
    let output = runtime.executor.query_switchable_branches(repository)?;
    let branch = parse_branch_records(&output.stdout)?
        .into_iter()
        .find(|branch| branch.full_name == request.full_name)
        .ok_or(BranchSwitchError::TargetChanged)?;
    if branch.kind != RepositoryBranchKind::Local
        || branch.symbolic_target.is_some()
        || branch.current
        || branch.oid != request.expected_oid
    {
        return Err(BranchSwitchError::TargetChanged);
    }
    Ok(branch)
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

fn outcome_from_push(push: &app_domain::PushStashResult) -> AutoStashOutcome {
    let create = match push.state {
        StashPushState::NoChanges => AutoStashCreateState::NotNeeded,
        StashPushState::Created => AutoStashCreateState::Created,
        StashPushState::Failed => AutoStashCreateState::Failed,
        StashPushState::Partial => AutoStashCreateState::Partial,
    };
    let stash_retained = push.stash.is_some();
    AutoStashOutcome {
        create,
        stash: push.stash.clone(),
        restore: if push.state == StashPushState::Partial {
            StashRestoreState::SkippedUnsafe
        } else {
            StashRestoreState::NotRequired
        },
        cleanup: if stash_retained {
            StashCleanupState::Retained
        } else {
            StashCleanupState::NotRequired
        },
        create_error: push.error_message.clone(),
        restore_error: None,
        cleanup_error: None,
    }
}

fn restore_after_successful_switch<E>(
    runtime: &RepositoryRuntime<E>,
    repository: &Path,
    stash: &StashIdentity,
    outcome: &mut AutoStashOutcome,
) where
    E: BranchActionGitExecutor + StashActionGitExecutor,
{
    match repository_status(&runtime.executor, repository) {
        Ok(status) => restore_stash(runtime, repository, stash, &status, outcome),
        Err(error) => {
            outcome.restore = StashRestoreState::Failed;
            outcome.cleanup = StashCleanupState::Retained;
            outcome.restore_error = Some(error.to_string());
        }
    }
}

fn restore_after_failed_switch<E>(
    runtime: &RepositoryRuntime<E>,
    repository: &Path,
    source: &RepositoryStatus,
    stash: &StashIdentity,
    outcome: &mut AutoStashOutcome,
) where
    E: BranchActionGitExecutor + StashActionGitExecutor,
{
    let current = match repository_status(&runtime.executor, repository) {
        Ok(status) => status,
        Err(error) => {
            outcome.restore = StashRestoreState::SkippedUnsafe;
            outcome.cleanup = StashCleanupState::Retained;
            outcome.restore_error = Some(error.to_string());
            return;
        }
    };
    let source_unchanged = current.branch.oid == source.branch.oid
        && current.branch.head == source.branch.head
        && current.branch.detached == source.branch.detached
        && current.branch.unborn == source.branch.unborn
        && current.entries.is_empty();
    if !source_unchanged {
        outcome.restore = StashRestoreState::SkippedUnsafe;
        outcome.cleanup = StashCleanupState::Retained;
        outcome.restore_error =
            Some("the branch state changed while switching; the auto-stash was kept".to_owned());
        return;
    }
    restore_stash(runtime, repository, stash, &current, outcome);
}

fn restore_stash<E>(
    runtime: &RepositoryRuntime<E>,
    repository: &Path,
    stash: &StashIdentity,
    status: &RepositoryStatus,
    outcome: &mut AutoStashOutcome,
) where
    E: BranchActionGitExecutor + StashActionGitExecutor,
{
    match runtime.pop_stash(
        repository,
        &PopStashRequest {
            stash: stash.clone(),
            restore_index: true,
            precondition: precondition_from_status(status),
        },
    ) {
        Ok(pop) => {
            outcome.restore = pop.restore;
            outcome.cleanup = pop.cleanup;
            outcome.restore_error = pop.restore_error;
            outcome.cleanup_error = pop.cleanup_error;
        }
        Err(error) => {
            outcome.restore = StashRestoreState::Failed;
            outcome.cleanup = StashCleanupState::Retained;
            outcome.restore_error = Some(error.to_string());
        }
    }
}

fn valid_stash_message(message: &str) -> bool {
    message.starts_with("WIP ")
        && message.len() <= MAX_STASH_MESSAGE_BYTES
        && !message.chars().any(char::is_control)
}

pub(crate) fn validate_requested_ref(full_name: &str) -> Result<(), BranchSwitchError> {
    if full_name.starts_with("refs/remotes/") {
        return Err(BranchSwitchError::RemoteBranchNotAllowed);
    }

    let Some(name) = full_name.strip_prefix("refs/heads/") else {
        return Err(BranchSwitchError::InvalidLocalRef);
    };
    if !is_valid_branch_name(name) {
        return Err(BranchSwitchError::InvalidLocalRef);
    }
    Ok(())
}

/// Mirrors the safety-relevant constraints from `git check-ref-format --branch` locally so no
/// attacker-controlled validation command has to run before the exact ref lookup.
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

fn result_from_branch(branch: &RepositoryBranch, changed: bool) -> SwitchBranchResult {
    SwitchBranchResult {
        full_name: branch.full_name.clone(),
        name: branch.name.clone(),
        head: branch.oid.clone(),
        changed,
        stash_created: false,
        operation_succeeded: true,
        operation_error: None,
        auto_stash: AutoStashOutcome::not_requested(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use git_core::GitExecutor;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedCall {
        Query,
        Status,
        Switch(String),
    }

    struct RecordingExecutor {
        branch_outputs: Mutex<VecDeque<Vec<u8>>>,
        status_output: Vec<u8>,
        switch_error: Option<String>,
        calls: Mutex<Vec<RecordedCall>>,
    }

    impl GitExecutor for RecordingExecutor {
        fn execute(
            &self,
            _repository: &Path,
            _arguments: &[&str],
        ) -> Result<GitOutput, GitRunError> {
            panic!("branch action tests must use the explicit policy API")
        }
    }

    impl BranchActionGitExecutor for RecordingExecutor {
        fn query_switchable_branches(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(RecordedCall::Query);
            Ok(GitOutput {
                stdout: self
                    .branch_outputs
                    .lock()
                    .expect("branch outputs lock")
                    .pop_front()
                    .expect("recorded branch output"),
                stderr: Vec::new(),
            })
        }

        fn switch_existing_local_branch(
            &self,
            _repository: &Path,
            branch_name: &str,
        ) -> Result<GitOutput, GitRunError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(RecordedCall::Switch(branch_name.to_owned()));
            if let Some(stderr) = &self.switch_error {
                return Err(GitRunError::Unsuccessful {
                    code: Some(1),
                    stderr: stderr.clone(),
                });
            }
            Ok(GitOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    impl crate::RepositoryStatusGitExecutor for RecordingExecutor {
        fn execute_repository_status(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(RecordedCall::Status);
            Ok(GitOutput {
                stdout: self.status_output.clone(),
                stderr: Vec::new(),
            })
        }

        fn execute_index_entries(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    impl StashActionGitExecutor for RecordingExecutor {
        fn stash_snapshot(
            &self,
            _repository: &Path,
        ) -> Result<Vec<StashIdentity>, StashActionError> {
            Ok(Vec::new())
        }

        fn push_stash(
            &self,
            _repository: &Path,
            _message: &str,
            _include_untracked: bool,
        ) -> Result<GitOutput, GitRunError> {
            panic!("clean branch action unit tests must not stash")
        }

        fn apply_stash(
            &self,
            _repository: &Path,
            _oid: &str,
            _restore_index: bool,
        ) -> Result<GitOutput, GitRunError> {
            panic!("clean branch action unit tests must not apply a stash")
        }

        fn drop_stash(
            &self,
            _repository: &Path,
            _selector: &str,
        ) -> Result<GitOutput, GitRunError> {
            panic!("clean branch action unit tests must not drop a stash")
        }
    }

    fn runtime(branch_output: &[u8]) -> RepositoryRuntime<RecordingExecutor> {
        RepositoryRuntime::new(RecordingExecutor {
            branch_outputs: Mutex::new(VecDeque::from([
                branch_output.to_vec(),
                branch_output.to_vec(),
            ])),
            status_output: Vec::new(),
            switch_error: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn request(full_name: &str) -> SwitchBranchRequest {
        SwitchBranchRequest {
            full_name: full_name.to_owned(),
            expected_oid: if full_name == "refs/heads/main" {
                "aaaa".to_owned()
            } else {
                "bbbb".to_owned()
            },
            stash_on_dirty: false,
            stash_message: None,
        }
    }

    #[test]
    fn switches_an_exact_existing_local_ref_using_its_short_name() {
        let runtime = runtime(
            b"refs/heads/main\0main\0aaaa\0*\0\0\0\0\nrefs/heads/rb/safe\0rb/safe\0bbbb\0 \0\0\0\0\n",
        );

        let result = runtime
            .switch_branch(Path::new("/repo"), &request("refs/heads/rb/safe"))
            .expect("switch succeeds");

        assert_eq!(result.full_name, "refs/heads/rb/safe");
        assert_eq!(result.name, "rb/safe");
        assert_eq!(result.head, "bbbb");
        assert!(result.changed);
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[
                RecordedCall::Query,
                RecordedCall::Status,
                RecordedCall::Query,
                RecordedCall::Switch("rb/safe".to_owned())
            ]
        );
    }

    #[test]
    fn rejects_a_target_that_moves_between_preflight_and_switch() {
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            branch_outputs: Mutex::new(VecDeque::from([
                b"refs/heads/rb/safe\0rb/safe\0bbbb\0 \0\0\0\0\n".to_vec(),
                b"refs/heads/rb/safe\0rb/safe\0cccc\0 \0\0\0\0\n".to_vec(),
            ])),
            status_output: Vec::new(),
            switch_error: None,
            calls: Mutex::new(Vec::new()),
        });

        let error = runtime
            .switch_branch(Path::new("/repo"), &request("refs/heads/rb/safe"))
            .expect_err("moving target must be rejected");

        assert!(matches!(error, BranchSwitchError::TargetChanged));
        assert_eq!(
            runtime.executor.calls.lock().unwrap().as_slice(),
            &[
                RecordedCall::Query,
                RecordedCall::Status,
                RecordedCall::Query
            ]
        );
    }

    #[test]
    fn current_branch_is_a_no_op_after_fresh_discovery() {
        let runtime = runtime(b"refs/heads/main\0main\0aaaa\0*\0\0\0\0\n");

        let result = runtime
            .switch_branch(Path::new("/repo"), &request("refs/heads/main"))
            .expect("current branch is accepted");

        assert!(!result.changed);
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[RecordedCall::Query]
        );
    }

    #[test]
    fn rejects_remote_ref_without_querying_git() {
        let runtime = runtime(b"refs/remotes/origin/main\0origin/main\0aaaa\0 \0\0\0\0\n");

        let error = runtime
            .switch_branch(Path::new("/repo"), &request("refs/remotes/origin/main"))
            .expect_err("remote ref is rejected");

        assert!(matches!(error, BranchSwitchError::RemoteBranchNotAllowed));
        assert!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .is_empty()
        );
    }

    #[test]
    fn rejects_malicious_or_ambiguous_refs_without_querying_git() {
        for full_name in [
            "--discard-changes",
            "refs/heads/-discard-changes",
            "refs/heads/safe\nbranch",
            "refs/heads/topic..other",
            "refs/heads/topic.lock",
        ] {
            let runtime = runtime(&[]);
            let error = runtime
                .switch_branch(Path::new("/repo"), &request(full_name))
                .expect_err("unsafe ref is rejected");
            assert!(matches!(error, BranchSwitchError::InvalidLocalRef));
            assert!(
                runtime
                    .executor
                    .calls
                    .lock()
                    .expect("calls lock")
                    .is_empty()
            );
        }
    }

    #[test]
    fn accepts_unicode_whitespace_that_git_allows_in_a_local_ref() {
        let full_name = "refs/heads/feature/\u{00a0}topic";
        let branch_name = "feature/\u{00a0}topic";
        let output = format!("{full_name}\0{branch_name}\0bbbb\0 \0\0\0\0\n");
        let runtime = runtime(output.as_bytes());

        let result = runtime
            .switch_branch(Path::new("/repo"), &request(full_name))
            .expect("Git-compatible Unicode branch is accepted");

        assert_eq!(result.full_name, full_name);
        assert_eq!(result.name, branch_name);
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[
                RecordedCall::Query,
                RecordedCall::Status,
                RecordedCall::Query,
                RecordedCall::Switch(branch_name.to_owned()),
            ]
        );
    }

    #[test]
    fn rejects_missing_and_symbolic_branches_without_switching() {
        let missing = runtime(b"refs/heads/main\0main\0aaaa\0*\0\0\0\0\n");
        assert!(matches!(
            missing.switch_branch(Path::new("/repo"), &request("refs/heads/other")),
            Err(BranchSwitchError::BranchNotFound)
        ));

        let symbolic = runtime(b"refs/heads/alias\0alias\0aaaa\0 \0\0\0refs/heads/main\0\n");
        assert!(matches!(
            symbolic.switch_branch(Path::new("/repo"), &request("refs/heads/alias")),
            Err(BranchSwitchError::SymbolicBranchNotAllowed)
        ));
        assert_eq!(
            symbolic
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[RecordedCall::Query]
        );
    }

    #[test]
    fn propagates_dirty_worktree_switch_failure_without_retry_or_force() {
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            branch_outputs: Mutex::new(VecDeque::from([
                b"refs/heads/feature\0feature\0bbbb\0 \0\0\0\0\n".to_vec(),
                b"refs/heads/feature\0feature\0bbbb\0 \0\0\0\0\n".to_vec(),
            ])),
            status_output: Vec::new(),
            switch_error: Some(
                "Your local changes to the following files would be overwritten".to_owned(),
            ),
            calls: Mutex::new(Vec::new()),
        });

        let error = runtime
            .switch_branch(Path::new("/repo"), &request("refs/heads/feature"))
            .expect_err("dirty switch fails");

        assert!(matches!(
            error,
            BranchSwitchError::Git(GitRunError::Unsuccessful { code: Some(1), stderr })
                if stderr.contains("would be overwritten")
        ));
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[
                RecordedCall::Query,
                RecordedCall::Status,
                RecordedCall::Query,
                RecordedCall::Switch("feature".to_owned())
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn git_runner_uses_exact_bounded_mutating_argv() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let directory = tempfile::tempdir().expect("temp directory");
        let script = directory.path().join("fake-git");
        fs::write(&script, "#!/bin/sh\nprintf '%s\\0' \"$@\"\n").expect("write script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
            .expect("make script executable");
        let runner = GitRunner::new(script);

        let output = runner
            .switch_existing_local_branch(Path::new("/repo"), "rb/safe;echo nope")
            .expect("fake command succeeds");

        let arguments = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty())
            .map(|value| String::from_utf8(value.to_vec()).expect("utf8 argument"))
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "--no-pager",
                "-c",
                "core.fsmonitor=false",
                "-C",
                "/repo",
                "switch",
                "--no-guess",
                "--",
                "rb/safe;echo nope",
            ]
        );
    }

    #[test]
    fn switches_a_real_git_branch_containing_non_breaking_space() {
        use std::{fs, process::Command};

        let directory = tempfile::tempdir().expect("temp directory");
        let repository = directory.path().join("repository");
        fs::create_dir(&repository).expect("create repository");

        let git = |arguments: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args(arguments)
                .output()
                .expect("run git")
        };
        assert!(git(&["init", "-q"]).status.success());
        assert!(
            git(&["config", "user.email", "test@example.com"])
                .status
                .success()
        );
        assert!(git(&["config", "user.name", "Test User"]).status.success());
        fs::write(repository.join("tracked.txt"), "content\n").expect("write tracked file");
        assert!(git(&["add", "tracked.txt"]).status.success());
        assert!(git(&["commit", "-qm", "initial"]).status.success());

        let branch_name = "feature/\u{00a0}topic";
        let full_name = format!("refs/heads/{branch_name}");
        assert!(git(&["branch", branch_name]).status.success());
        let expected_oid = String::from_utf8(git(&["rev-parse", &full_name]).stdout)
            .expect("UTF-8 OID")
            .trim()
            .to_owned();

        let runtime = RepositoryRuntime::new(GitRunner::default());
        let result = runtime
            .switch_branch(
                &repository,
                &SwitchBranchRequest {
                    full_name: full_name.clone(),
                    expected_oid,
                    stash_on_dirty: false,
                    stash_message: None,
                },
            )
            .expect("switch Unicode branch");

        assert!(result.changed);
        assert_eq!(result.full_name, full_name);
        assert_eq!(result.name, branch_name);
        assert_eq!(
            String::from_utf8(git(&["symbolic-ref", "--short", "HEAD"]).stdout)
                .expect("UTF-8 HEAD")
                .trim(),
            branch_name
        );
    }
}
