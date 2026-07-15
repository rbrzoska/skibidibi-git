use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use app_domain::{
    ApplyStashRequest, ApplyStashResult, DropStashRequest, DropStashResult, PopStashRequest,
    PopStashResult, PushStashRequest, PushStashResult, RepositoryStatePrecondition,
    RepositoryStatus, StashCleanupState, StashIdentity, StashPushState, StashRestoreState,
    StatusEntryKind,
};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{
    RepositoryRuntime, RepositoryRuntimeError, RepositoryStatusGitExecutor, repository_status,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(120);
const DROP_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_LIMIT: usize = 512 * 1024;
const MAX_STASH_MESSAGE_BYTES: usize = 512;
const STASH_FORMAT: &str = "--format=%H%x00%gd%x00";
const MARKED_STASH_FORMAT: &str = "--format=%H%x00%gd%x00%gs%x00";
const PIN_REF_PREFIX: &str = "refs/skibidibi/stash-pins/";
static STASH_NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

pub trait StashActionGitExecutor: RepositoryStatusGitExecutor {
    fn stash_snapshot(&self, repository: &Path) -> Result<Vec<StashIdentity>, StashActionError>;

    fn push_stash(
        &self,
        repository: &Path,
        message: &str,
        include_untracked: bool,
    ) -> Result<GitOutput, GitRunError>;

    fn create_stash(&self, _repository: &Path, _message: &str) -> Result<GitOutput, GitRunError> {
        panic!("stash create is not implemented by this executor")
    }

    fn store_stash(
        &self,
        _repository: &Path,
        _message: &str,
        _oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        panic!("stash store is not implemented by this executor")
    }

    fn reset_stashed_worktree(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
        panic!("stash reset is not implemented by this executor")
    }

    fn pin_stash(
        &self,
        _repository: &Path,
        _pin_ref: &str,
        _oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        panic!("stash pinning is not implemented by this executor")
    }

    fn unpin_stash(
        &self,
        _repository: &Path,
        _pin_ref: &str,
        _oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        panic!("stash unpinning is not implemented by this executor")
    }

    fn stashes_with_marker(
        &self,
        _repository: &Path,
        _marker: &str,
    ) -> Result<Vec<StashIdentity>, StashActionError> {
        panic!("stash marker lookup is not implemented by this executor")
    }

    fn apply_stash(
        &self,
        repository: &Path,
        oid: &str,
        restore_index: bool,
    ) -> Result<GitOutput, GitRunError>;

    fn drop_stash(&self, repository: &Path, selector: &str) -> Result<GitOutput, GitRunError>;
}

impl StashActionGitExecutor for GitRunner {
    fn stash_snapshot(&self, repository: &Path) -> Result<Vec<StashIdentity>, StashActionError> {
        let presence = self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                [
                    "for-each-ref",
                    "--format=%(refname)",
                    "--count=1",
                    "refs/stash",
                ],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )?;
        if presence.stdout.is_empty() {
            return Ok(Vec::new());
        }
        let output = self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["log", "-g", STASH_FORMAT, "refs/stash"],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )?;
        parse_stash_snapshot(&output.stdout)
    }

    fn create_stash(&self, repository: &Path, message: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, ["stash", "create", message])
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn store_stash(
        &self,
        repository: &Path,
        message: &str,
        oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["stash", "store", "--message", message, oid],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn reset_stashed_worktree(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["reset", "--hard", "--quiet", "HEAD"],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn pin_stash(
        &self,
        repository: &Path,
        pin_ref: &str,
        oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, ["update-ref", pin_ref, oid])
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(DROP_TIMEOUT),
        )
    }

    fn unpin_stash(
        &self,
        repository: &Path,
        pin_ref: &str,
        oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["update-ref", "-d", pin_ref, oid],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(DROP_TIMEOUT),
        )
    }

    fn stashes_with_marker(
        &self,
        repository: &Path,
        marker: &str,
    ) -> Result<Vec<StashIdentity>, StashActionError> {
        let presence = self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                [
                    "for-each-ref",
                    "--format=%(refname)",
                    "--count=1",
                    "refs/stash",
                ],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )?;
        if presence.stdout.is_empty() {
            return Ok(Vec::new());
        }
        let output = self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["log", "-g", MARKED_STASH_FORMAT, "refs/stash"],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )?;
        parse_marked_stashes(&output.stdout, marker)
    }

    fn push_stash(
        &self,
        repository: &Path,
        message: &str,
        include_untracked: bool,
    ) -> Result<GitOutput, GitRunError> {
        let mut arguments = vec![OsString::from("stash"), OsString::from("push")];
        if include_untracked {
            arguments.push(OsString::from("--include-untracked"));
        }
        arguments.extend([OsString::from("--message"), OsString::from(message)]);
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, arguments)
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn apply_stash(
        &self,
        repository: &Path,
        oid: &str,
        restore_index: bool,
    ) -> Result<GitOutput, GitRunError> {
        let mut arguments = vec![OsString::from("stash"), OsString::from("apply")];
        if restore_index {
            arguments.push(OsString::from("--index"));
        }
        arguments.push(OsString::from(oid));
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, arguments)
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn drop_stash(&self, repository: &Path, selector: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, ["stash", "drop", selector])
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(DROP_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum StashActionError {
    #[error("the stash request is invalid")]
    InvalidRequest,
    #[error("repository state changed; refresh before retrying")]
    StaleState,
    #[error("conflicted entries must be resolved before this stash operation")]
    ConflictsPresent,
    #[error("stash apply and pop require a clean working tree")]
    DirtyWorkingTree,
    #[error("stashing requires an existing commit")]
    UnbornHead,
    #[error("the requested stash is no longer available")]
    StashNotFound,
    #[error("the stash OID is ambiguous in the stash reflog")]
    AmbiguousStash,
    #[error("Git reported success but the created stash could not be identified")]
    MissingCreatedStash,
    #[error("stash output is invalid")]
    InvalidOutput,
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
}

impl StashActionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::InvalidOutput => "invalidRequest",
            Self::StaleState | Self::StashNotFound => "staleState",
            Self::ConflictsPresent => "conflictsPresent",
            Self::DirtyWorkingTree => "dirtyWorkingTree",
            Self::UnbornHead => "unbornHead",
            Self::AmbiguousStash => "ambiguousStash",
            Self::MissingCreatedStash | Self::Repository(_) => "internal",
            Self::Git(GitRunError::TimedOut { .. }) => "timedOut",
            Self::Git(GitRunError::OutputLimitExceeded { .. }) => "outputLimit",
            Self::Git(_) => "gitRejected",
        }
    }
}

impl<E: StashActionGitExecutor> RepositoryRuntime<E> {
    pub fn push_stash(
        &self,
        repository: &Path,
        request: &PushStashRequest,
    ) -> Result<PushStashResult, StashActionError> {
        validate_message(&request.message)?;
        let before_status = repository_status(&self.executor, repository)?;
        validate_precondition(&before_status, &request.precondition)?;
        reject_conflicts(&before_status)?;
        if before_status.branch.unborn {
            return Err(StashActionError::UnbornHead);
        }
        let has_stashable_changes = before_status
            .entries
            .iter()
            .any(|entry| request.include_untracked || entry.kind != StatusEntryKind::Untracked);
        if !has_stashable_changes {
            return Ok(PushStashResult {
                state: StashPushState::NoChanges,
                stash: None,
                status: Some(before_status),
                error_message: None,
                mutation_oid: None,
                mutation_may_have_occurred: false,
            });
        }

        let before_stashes = self.executor.stash_snapshot(repository)?;
        if request.include_untracked {
            push_with_untracked(&self.executor, repository, &request.message)
        } else {
            push_tracked_exact(
                &self.executor,
                repository,
                &request.message,
                &before_stashes,
                &request.precondition,
            )
        }
    }

    pub fn apply_stash(
        &self,
        repository: &Path,
        request: &ApplyStashRequest,
    ) -> Result<ApplyStashResult, StashActionError> {
        let before = prepare_restore(
            &self.executor,
            repository,
            &request.stash,
            &request.precondition,
        )?;
        debug_assert!(before.entries.is_empty());
        let (restore, status, error_message) = apply_and_classify(
            &self.executor,
            repository,
            &request.stash.oid,
            request.restore_index,
        );
        Ok(ApplyStashResult {
            stash: request.stash.clone(),
            restore,
            cleanup: StashCleanupState::Retained,
            status,
            error_message,
            mutation_may_have_occurred: true,
        })
    }

    pub fn pop_stash(
        &self,
        repository: &Path,
        request: &PopStashRequest,
    ) -> Result<PopStashResult, StashActionError> {
        prepare_restore(
            &self.executor,
            repository,
            &request.stash,
            &request.precondition,
        )?;
        let (restore, status, restore_error) = apply_and_classify(
            &self.executor,
            repository,
            &request.stash.oid,
            request.restore_index,
        );
        if restore != StashRestoreState::Applied {
            return Ok(PopStashResult {
                stash: request.stash.clone(),
                restore,
                cleanup: StashCleanupState::Retained,
                status,
                restore_error,
                cleanup_error: None,
                mutation_may_have_occurred: true,
            });
        }

        let cleanup = drop_exact(&self.executor, repository, &request.stash.oid);
        Ok(PopStashResult {
            stash: request.stash.clone(),
            restore,
            cleanup: cleanup.state,
            status,
            restore_error: None,
            cleanup_error: cleanup.error,
            mutation_may_have_occurred: true,
        })
    }

    pub fn drop_stash(
        &self,
        repository: &Path,
        request: &DropStashRequest,
    ) -> Result<DropStashResult, StashActionError> {
        validate_identity(&request.stash)?;
        ensure_unique_stash(
            &self.executor.stash_snapshot(repository)?,
            &request.stash.oid,
        )?;
        let cleanup = drop_exact(&self.executor, repository, &request.stash.oid);
        Ok(DropStashResult {
            stash: request.stash.clone(),
            cleanup: cleanup.state,
            error_message: cleanup.error,
            mutation_may_have_occurred: cleanup.mutation_may_have_occurred,
        })
    }
}

fn push_tracked_exact<E: StashActionGitExecutor>(
    executor: &E,
    repository: &Path,
    message: &str,
    before: &[StashIdentity],
    expected: &RepositoryStatePrecondition,
) -> Result<PushStashResult, StashActionError> {
    let created = match executor.create_stash(repository, message) {
        Ok(output) => match parse_created_oid(&output.stdout) {
            Ok(oid) => oid,
            Err(error) => {
                let (status, status_error) = status_after_mutation(executor, repository);
                return Ok(PushStashResult {
                    state: StashPushState::Failed,
                    stash: None,
                    status,
                    error_message: join_errors(Some(error.to_string()), status_error),
                    mutation_oid: None,
                    mutation_may_have_occurred: false,
                });
            }
        },
        Err(error) => {
            let (status, status_error) = status_after_mutation(executor, repository);
            return Ok(PushStashResult {
                state: StashPushState::Failed,
                stash: None,
                status,
                error_message: join_errors(Some(error.to_string()), status_error),
                mutation_oid: None,
                mutation_may_have_occurred: false,
            });
        }
    };

    let pin_ref = pin_ref(&created);
    if let Err(error) = executor.pin_stash(repository, &pin_ref, &created) {
        let (status, status_error) = status_after_mutation(executor, repository);
        return Ok(PushStashResult {
            state: StashPushState::Partial,
            stash: None,
            status,
            error_message: join_errors(Some(error.to_string()), status_error),
            mutation_oid: Some(created),
            mutation_may_have_occurred: true,
        });
    }

    let store_result = executor.store_stash(repository, message, &created);
    let (after, snapshot_error) = snapshot_after_mutation(executor, repository);
    let identity = after.as_deref().and_then(|snapshot| {
        (!before.iter().any(|stash| stash.oid == created))
            .then(|| unique_identity(snapshot, &created))
            .flatten()
            .cloned()
    });
    let (pre_reset_status, pre_reset_status_error) = status_after_mutation(executor, repository);
    let state_unchanged = pre_reset_status
        .as_ref()
        .is_some_and(|status| validate_precondition(status, expected).is_ok());
    let reset_result = (identity.is_some() && state_unchanged)
        .then(|| executor.reset_stashed_worktree(repository));
    let (status, status_error) = if reset_result.is_some() {
        status_after_mutation(executor, repository)
    } else {
        (pre_reset_status, None)
    };
    let mut error_message = join_errors(
        store_result.err().map(|error| error.to_string()),
        snapshot_error,
    );
    if identity.is_none() {
        error_message = join_errors(
            error_message,
            Some(
                "the exact stored stash could not be rebound; the working tree was not reset"
                    .to_owned(),
            ),
        );
    }
    if !state_unchanged {
        error_message = join_errors(
            error_message,
            Some(
                "repository state changed after stash creation; the working tree was not reset"
                    .to_owned(),
            ),
        );
    }
    let reset_succeeded = matches!(reset_result, Some(Ok(_)));
    if let Some(Err(error)) = reset_result {
        error_message = join_errors(error_message, Some(error.to_string()));
    }
    let mut final_identity = identity;
    let mut pin_released = false;
    if reset_succeeded {
        match executor.stash_snapshot(repository) {
            Ok(snapshot) => {
                final_identity = unique_identity(&snapshot, &created).cloned();
                if final_identity.is_some() {
                    match executor.unpin_stash(repository, &pin_ref, &created) {
                        Ok(_) => pin_released = true,
                        Err(error) => {
                            error_message = join_errors(
                                error_message,
                                Some(format!("stash pin cleanup failed: {error}")),
                            );
                        }
                    }
                } else {
                    error_message = join_errors(
                        error_message,
                        Some(
                            "the stored stash disappeared after reset; its safety pin was retained"
                                .to_owned(),
                        ),
                    );
                }
            }
            Err(error) => {
                error_message = join_errors(
                    error_message,
                    Some(format!(
                        "post-reset stash verification failed; its safety pin was retained: {error}"
                    )),
                );
            }
        }
    }
    error_message = join_errors(
        error_message,
        join_errors(pre_reset_status_error, status_error),
    );
    let complete = final_identity.is_some()
        && reset_succeeded
        && pin_released
        && error_message.is_none()
        && status.is_some();
    Ok(PushStashResult {
        state: if complete {
            StashPushState::Created
        } else {
            StashPushState::Partial
        },
        stash: final_identity,
        status,
        error_message,
        mutation_oid: Some(created),
        mutation_may_have_occurred: true,
    })
}

fn push_with_untracked<E: StashActionGitExecutor>(
    executor: &E,
    repository: &Path,
    message: &str,
) -> Result<PushStashResult, StashActionError> {
    // `git stash create` has no include-untracked mode. Keep Git's porcelain operation for this
    // case, then attribute conservatively: more than one new OID is never guessed.
    let marker = unique_stash_marker();
    let marked_message = format!("{message} [{marker}]");
    let push_result = executor.push_stash(repository, &marked_message, true);
    let (marked, marker_error) = match executor.stashes_with_marker(repository, &marker) {
        Ok(matches) => (Some(matches), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let created = marked.as_deref().and_then(single_identity).cloned();
    let (status, status_error) = status_after_mutation(executor, repository);
    let mut error_message = join_errors(
        push_result.err().map(|error| error.to_string()),
        marker_error,
    );
    if created.is_none() {
        error_message = join_errors(
            error_message,
            Some(
                "the include-untracked stash could not be attributed to one exact new OID"
                    .to_owned(),
            ),
        );
    }
    error_message = join_errors(error_message, status_error);
    let complete = created.is_some() && error_message.is_none() && status.is_some();
    Ok(PushStashResult {
        state: if complete {
            StashPushState::Created
        } else {
            StashPushState::Partial
        },
        mutation_oid: created.as_ref().map(|stash| stash.oid.clone()),
        stash: created,
        status,
        error_message,
        mutation_may_have_occurred: true,
    })
}

fn parse_created_oid(output: &[u8]) -> Result<String, StashActionError> {
    let value = std::str::from_utf8(output)
        .map_err(|_| StashActionError::InvalidOutput)?
        .trim();
    valid_oid(value)
        .then(|| value.to_owned())
        .ok_or(StashActionError::InvalidOutput)
}

fn parse_dropped_oid(output: &[u8]) -> Option<String> {
    let output = std::str::from_utf8(output).ok()?.trim();
    let oid = output.rsplit_once('(')?.1.strip_suffix(')')?;
    valid_oid(oid).then(|| oid.to_owned())
}

fn unique_stash_marker() -> String {
    let counter = STASH_NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!(
        "skibidibi-git:{:x}:{counter:x}:{nanos:x}",
        std::process::id()
    )
}

fn pin_ref(oid: &str) -> String {
    format!("{PIN_REF_PREFIX}{oid}")
}

fn single_identity(stashes: &[StashIdentity]) -> Option<&StashIdentity> {
    let [stash] = stashes else {
        return None;
    };
    Some(stash)
}

fn snapshot_after_mutation<E: StashActionGitExecutor>(
    executor: &E,
    repository: &Path,
) -> (Option<Vec<StashIdentity>>, Option<String>) {
    match executor.stash_snapshot(repository) {
        Ok(snapshot) => (Some(snapshot), None),
        Err(error) => (None, Some(error.to_string())),
    }
}

fn status_after_mutation<E: StashActionGitExecutor>(
    executor: &E,
    repository: &Path,
) -> (Option<RepositoryStatus>, Option<String>) {
    match repository_status(executor, repository) {
        Ok(status) => (Some(status), None),
        Err(error) => (None, Some(error.to_string())),
    }
}

fn join_errors(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(error), None) | (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}

fn unique_identity<'a>(snapshot: &'a [StashIdentity], oid: &str) -> Option<&'a StashIdentity> {
    let mut matches = snapshot.iter().filter(|stash| stash.oid == oid);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn prepare_restore<E: StashActionGitExecutor>(
    executor: &E,
    repository: &Path,
    stash: &StashIdentity,
    precondition: &RepositoryStatePrecondition,
) -> Result<RepositoryStatus, StashActionError> {
    validate_identity(stash)?;
    let status = repository_status(executor, repository)?;
    validate_precondition(&status, precondition)?;
    reject_conflicts(&status)?;
    if !status.entries.is_empty() {
        return Err(StashActionError::DirtyWorkingTree);
    }
    ensure_unique_stash(&executor.stash_snapshot(repository)?, &stash.oid)?;
    Ok(status)
}

fn apply_and_classify<E: StashActionGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
    restore_index: bool,
) -> (StashRestoreState, Option<RepositoryStatus>, Option<String>) {
    let result = executor.apply_stash(repository, oid, restore_index);
    let (status, status_error) = status_after_mutation(executor, repository);
    match (result, status.as_ref()) {
        (Ok(_), Some(_)) => (StashRestoreState::Applied, status, status_error),
        (Ok(_), None) => (StashRestoreState::Failed, None, status_error),
        (Err(error), status) => {
            let state = if status.is_some_and(|status| {
                status
                    .entries
                    .iter()
                    .any(|entry| entry.kind == StatusEntryKind::Unmerged)
            }) {
                StashRestoreState::Conflicted
            } else {
                StashRestoreState::Failed
            };
            (
                state,
                status.cloned(),
                join_errors(Some(error.to_string()), status_error),
            )
        }
    }
}

struct CleanupAttempt {
    state: StashCleanupState,
    error: Option<String>,
    mutation_may_have_occurred: bool,
}

fn drop_exact<E: StashActionGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
) -> CleanupAttempt {
    let before = match executor.stash_snapshot(repository) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return CleanupAttempt {
                state: StashCleanupState::Failed,
                error: Some(error.to_string()),
                mutation_may_have_occurred: false,
            };
        }
    };
    let rebound = match ensure_unique_stash(&before, oid) {
        Ok(stash) => stash,
        Err(error) => {
            return CleanupAttempt {
                state: StashCleanupState::Failed,
                error: Some(error.to_string()),
                mutation_may_have_occurred: false,
            };
        }
    };
    let drop_result = executor.drop_stash(repository, &rebound.selector);
    let reported_oid = drop_result
        .as_ref()
        .ok()
        .and_then(|output| parse_dropped_oid(&output.stdout));
    if let Some(reported_oid) = reported_oid.as_deref().filter(|reported| *reported != oid) {
        let recovery_result = executor.store_stash(
            repository,
            "Skibidibi Git recovery after selector race",
            reported_oid,
        );
        let verification = executor.stash_snapshot(repository);
        let recovered = verification
            .as_ref()
            .is_ok_and(|snapshot| snapshot.iter().any(|stash| stash.oid == reported_oid));
        return CleanupAttempt {
            state: StashCleanupState::Failed,
            error: join_errors(
                Some(format!(
                    "Git dropped foreign stash {reported_oid} instead of {oid}; recovery was attempted"
                )),
                join_errors(
                    recovery_result.err().map(|error| error.to_string()),
                    verification
                        .err()
                        .map(|error| error.to_string())
                        .or_else(|| {
                            (!recovered)
                                .then(|| "foreign stash recovery could not be verified".to_owned())
                        }),
                ),
            ),
            mutation_may_have_occurred: true,
        };
    }
    let after = match executor.stash_snapshot(repository) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return CleanupAttempt {
                state: StashCleanupState::Failed,
                error: join_errors(
                    drop_result.err().map(|error| error.to_string()),
                    Some(format!("post-drop reconciliation failed: {error}")),
                ),
                mutation_may_have_occurred: true,
            };
        }
    };

    let before_counts = oid_counts(&before);
    let after_counts = oid_counts(&after);
    let unexpected_removed = before_counts
        .iter()
        .filter_map(|(candidate, before_count)| {
            if candidate == oid {
                return None;
            }
            let missing = before_count.saturating_sub(*after_counts.get(candidate).unwrap_or(&0));
            (missing > 0).then(|| (candidate.clone(), missing))
        })
        .collect::<Vec<_>>();
    let unexpected_added = after_counts
        .iter()
        .any(|(candidate, after_count)| *after_count > *before_counts.get(candidate).unwrap_or(&0));
    if !unexpected_removed.is_empty() {
        let mut recovery_error = None;
        for (removed_oid, count) in &unexpected_removed {
            for _ in 0..*count {
                if let Err(error) = executor.store_stash(
                    repository,
                    "Skibidibi Git recovery after concurrent stash change",
                    removed_oid,
                ) {
                    recovery_error = join_errors(recovery_error, Some(error.to_string()));
                }
            }
        }
        let verification_error = match executor.stash_snapshot(repository) {
            Ok(recovered) => {
                let recovered_counts = oid_counts(&recovered);
                unexpected_removed.iter().find_map(|(removed_oid, _)| {
                    (recovered_counts.get(removed_oid).unwrap_or(&0)
                        < before_counts.get(removed_oid).unwrap_or(&0))
                    .then(|| format!("failed to recover foreign stash {removed_oid}"))
                })
            }
            Err(error) => Some(format!("stash recovery could not be verified: {error}")),
        };
        return CleanupAttempt {
            state: StashCleanupState::Failed,
            error: join_errors(
                Some(
                    "a concurrent reflog shift removed a foreign stash; recovery was attempted"
                        .to_owned(),
                ),
                join_errors(recovery_error, verification_error),
            ),
            mutation_may_have_occurred: true,
        };
    }

    let target_absent = !after_counts.contains_key(oid);
    if target_absent {
        let race_notice = unexpected_added.then(|| {
            "concurrent stash additions were detected during cleanup; the exact target is absent"
                .to_owned()
        });
        let mut error = join_errors(
            drop_result.err().map(|error| error.to_string()),
            race_notice,
        );
        if reported_oid.as_deref() != Some(oid) {
            error = join_errors(
                error,
                Some("Git did not report the exact dropped OID; cleanup is not trusted".to_owned()),
            );
            return CleanupAttempt {
                state: StashCleanupState::Failed,
                error,
                mutation_may_have_occurred: true,
            };
        }
        if let Err(unpin_error) = executor.unpin_stash(repository, &pin_ref(oid), oid) {
            error = join_errors(
                error,
                Some(format!("stash pin cleanup failed: {unpin_error}")),
            );
        }
        return CleanupAttempt {
            state: StashCleanupState::Dropped,
            error,
            mutation_may_have_occurred: true,
        };
    }
    CleanupAttempt {
        state: StashCleanupState::Failed,
        error: join_errors(
            drop_result.err().map(|error| error.to_string()),
            Some(
                "the exact stash remains after cleanup; no foreign deletion was accepted"
                    .to_owned(),
            ),
        ),
        mutation_may_have_occurred: true,
    }
}

fn oid_counts(snapshot: &[StashIdentity]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for stash in snapshot {
        *counts.entry(stash.oid.clone()).or_default() += 1;
    }
    counts
}

fn ensure_unique_stash<'a>(
    snapshot: &'a [StashIdentity],
    oid: &str,
) -> Result<&'a StashIdentity, StashActionError> {
    let matches = snapshot
        .iter()
        .filter(|stash| stash.oid == oid)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [stash] => Ok(stash),
        [] => Err(StashActionError::StashNotFound),
        _ => Err(StashActionError::AmbiguousStash),
    }
}

fn validate_precondition(
    status: &RepositoryStatus,
    expected: &RepositoryStatePrecondition,
) -> Result<(), StashActionError> {
    if expected.expected_index_fingerprint.is_empty()
        || expected.expected_worktree_fingerprint.is_empty()
        || status.branch.oid != expected.expected_head
        || status.branch.head != expected.expected_head_name
        || status.branch.detached != expected.expected_detached
        || status.branch.unborn != expected.expected_unborn
        || status.index_fingerprint != expected.expected_index_fingerprint
        || status.worktree_fingerprint != expected.expected_worktree_fingerprint
    {
        return Err(StashActionError::StaleState);
    }
    Ok(())
}

fn reject_conflicts(status: &RepositoryStatus) -> Result<(), StashActionError> {
    if status
        .entries
        .iter()
        .any(|entry| entry.kind == StatusEntryKind::Unmerged)
    {
        return Err(StashActionError::ConflictsPresent);
    }
    Ok(())
}

fn validate_message(message: &str) -> Result<(), StashActionError> {
    if message.trim().is_empty()
        || message.len() > MAX_STASH_MESSAGE_BYTES
        || message.chars().any(char::is_control)
    {
        return Err(StashActionError::InvalidRequest);
    }
    Ok(())
}

fn validate_identity(stash: &StashIdentity) -> Result<(), StashActionError> {
    if !valid_oid(&stash.oid) || !valid_selector(&stash.selector) {
        return Err(StashActionError::InvalidRequest);
    }
    Ok(())
}

fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64)
        && oid
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_selector(selector: &str) -> bool {
    selector
        .strip_prefix("stash@{")
        .and_then(|value| value.strip_suffix('}'))
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
}

fn parse_stash_snapshot(input: &[u8]) -> Result<Vec<StashIdentity>, StashActionError> {
    let mut result = Vec::new();
    for record in input.split(|byte| *byte == b'\n') {
        if record.is_empty() {
            continue;
        }
        let fields = record.split(|byte| *byte == 0).collect::<Vec<_>>();
        let [oid, selector, trailing] = fields.as_slice() else {
            return Err(StashActionError::InvalidOutput);
        };
        if !trailing.is_empty() {
            return Err(StashActionError::InvalidOutput);
        }
        let identity = StashIdentity {
            oid: std::str::from_utf8(oid)
                .map_err(|_| StashActionError::InvalidOutput)?
                .to_owned(),
            selector: std::str::from_utf8(selector)
                .map_err(|_| StashActionError::InvalidOutput)?
                .to_owned(),
        };
        validate_identity(&identity)?;
        result.push(identity);
    }
    Ok(result)
}

fn parse_marked_stashes(
    input: &[u8],
    marker: &str,
) -> Result<Vec<StashIdentity>, StashActionError> {
    let needle = format!("[{marker}]");
    let mut result = Vec::new();
    for record in input.split(|byte| *byte == b'\n') {
        if record.is_empty() {
            continue;
        }
        let fields = record.split(|byte| *byte == 0).collect::<Vec<_>>();
        let [oid, selector, subject, trailing] = fields.as_slice() else {
            return Err(StashActionError::InvalidOutput);
        };
        if !trailing.is_empty() {
            return Err(StashActionError::InvalidOutput);
        }
        let subject = std::str::from_utf8(subject).map_err(|_| StashActionError::InvalidOutput)?;
        if !subject.contains(&needle) {
            continue;
        }
        let identity = StashIdentity {
            oid: std::str::from_utf8(oid)
                .map_err(|_| StashActionError::InvalidOutput)?
                .to_owned(),
            selector: std::str::from_utf8(selector)
                .map_err(|_| StashActionError::InvalidOutput)?
                .to_owned(),
        };
        validate_identity(&identity)?;
        result.push(identity);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Mutex, atomic::AtomicUsize},
    };

    use super::*;

    struct RacingDropExecutor {
        snapshots: Mutex<VecDeque<Result<Vec<StashIdentity>, StashActionError>>>,
        recovered: Mutex<Vec<String>>,
        dropped_oid: String,
    }

    struct PartialCreateExecutor {
        oid: String,
        stored: Mutex<Vec<String>>,
        snapshots: Mutex<VecDeque<Result<Vec<StashIdentity>, StashActionError>>>,
        status_calls: AtomicUsize,
        drift_on_second_status: bool,
        reset_called: Mutex<bool>,
    }

    impl RepositoryStatusGitExecutor for PartialCreateExecutor {
        fn execute_repository_status(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            let call = self.status_calls.fetch_add(1, Ordering::Relaxed);
            let drift = self.drift_on_second_status && call > 0;
            Ok(GitOutput {
                stdout: format!(
                    "# branch.oid {}\0# branch.head main\0{}",
                    "d".repeat(40),
                    if drift {
                        "? changed-after-create.txt\0"
                    } else {
                        ""
                    }
                )
                .into_bytes(),
                stderr: Vec::new(),
            })
        }

        fn execute_index_entries(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            Ok(empty_output())
        }
    }

    impl StashActionGitExecutor for PartialCreateExecutor {
        fn stash_snapshot(
            &self,
            _repository: &Path,
        ) -> Result<Vec<StashIdentity>, StashActionError> {
            self.snapshots
                .lock()
                .unwrap()
                .pop_front()
                .expect("expected create snapshot")
        }

        fn push_stash(
            &self,
            _repository: &Path,
            _message: &str,
            _include_untracked: bool,
        ) -> Result<GitOutput, GitRunError> {
            panic!("exact tracked creation does not use porcelain push")
        }

        fn create_stash(
            &self,
            _repository: &Path,
            _message: &str,
        ) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: format!("{}\n", self.oid).into_bytes(),
                stderr: Vec::new(),
            })
        }

        fn store_stash(
            &self,
            _repository: &Path,
            _message: &str,
            oid: &str,
        ) -> Result<GitOutput, GitRunError> {
            self.stored.lock().unwrap().push(oid.to_owned());
            Ok(empty_output())
        }

        fn pin_stash(
            &self,
            _repository: &Path,
            _pin_ref: &str,
            _oid: &str,
        ) -> Result<GitOutput, GitRunError> {
            Ok(empty_output())
        }

        fn reset_stashed_worktree(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            *self.reset_called.lock().unwrap() = true;
            Ok(empty_output())
        }

        fn apply_stash(
            &self,
            _repository: &Path,
            _oid: &str,
            _restore_index: bool,
        ) -> Result<GitOutput, GitRunError> {
            panic!("exact creation test does not apply")
        }

        fn drop_stash(
            &self,
            _repository: &Path,
            _selector: &str,
        ) -> Result<GitOutput, GitRunError> {
            panic!("exact creation test does not drop")
        }
    }

    impl RepositoryStatusGitExecutor for RacingDropExecutor {
        fn execute_repository_status(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            panic!("drop reconciliation does not query status")
        }

        fn execute_index_entries(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            panic!("drop reconciliation does not query the index")
        }
    }

    impl StashActionGitExecutor for RacingDropExecutor {
        fn stash_snapshot(
            &self,
            _repository: &Path,
        ) -> Result<Vec<StashIdentity>, StashActionError> {
            self.snapshots
                .lock()
                .expect("snapshots lock")
                .pop_front()
                .expect("expected snapshot")
        }

        fn push_stash(
            &self,
            _repository: &Path,
            _message: &str,
            _include_untracked: bool,
        ) -> Result<GitOutput, GitRunError> {
            panic!("drop reconciliation does not push")
        }

        fn store_stash(
            &self,
            _repository: &Path,
            _message: &str,
            oid: &str,
        ) -> Result<GitOutput, GitRunError> {
            self.recovered
                .lock()
                .expect("recovery lock")
                .push(oid.to_owned());
            Ok(empty_output())
        }

        fn apply_stash(
            &self,
            _repository: &Path,
            _oid: &str,
            _restore_index: bool,
        ) -> Result<GitOutput, GitRunError> {
            panic!("drop reconciliation does not apply")
        }

        fn drop_stash(
            &self,
            _repository: &Path,
            _selector: &str,
        ) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: format!("Dropped stash@{{0}} ({})\n", self.dropped_oid).into_bytes(),
                stderr: Vec::new(),
            })
        }

        fn unpin_stash(
            &self,
            _repository: &Path,
            _pin_ref: &str,
            _oid: &str,
        ) -> Result<GitOutput, GitRunError> {
            Ok(empty_output())
        }
    }

    fn empty_output() -> GitOutput {
        GitOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    fn identity(character: char, selector: &str) -> StashIdentity {
        StashIdentity {
            oid: character.to_string().repeat(40),
            selector: selector.to_owned(),
        }
    }

    #[test]
    fn parses_snapshot_and_rejects_unsafe_identity_fields() {
        let oid = "a".repeat(40);
        let bytes = format!("{oid}\0stash@{{0}}\0\n");
        assert_eq!(
            parse_stash_snapshot(bytes.as_bytes()).unwrap(),
            vec![StashIdentity {
                oid,
                selector: "stash@{0}".to_owned(),
            }]
        );
        assert!(parse_stash_snapshot(b"deadbeef\0stash@{0}\0\n").is_err());
        assert!(
            parse_stash_snapshot(format!("{}\0--help\0\n", "a".repeat(40)).as_bytes()).is_err()
        );
    }

    #[test]
    fn rebinds_a_unique_oid_and_rejects_duplicates() {
        let oid = "b".repeat(40);
        let one = vec![StashIdentity {
            oid: oid.clone(),
            selector: "stash@{3}".to_owned(),
        }];
        assert_eq!(
            ensure_unique_stash(&one, &oid).unwrap().selector,
            "stash@{3}"
        );
        let duplicate = vec![
            one[0].clone(),
            StashIdentity {
                oid: oid.clone(),
                selector: "stash@{4}".to_owned(),
            },
        ];
        assert!(matches!(
            ensure_unique_stash(&duplicate, &oid),
            Err(StashActionError::AmbiguousStash)
        ));
    }

    #[test]
    fn marker_attribution_ignores_unrelated_concurrent_stashes() {
        let marker = "skibidibi-git:1:2:3";
        let input = format!(
            "{}\0stash@{{0}}\0On main: unrelated\0\n{}\0stash@{{1}}\0On main: WIP [{marker}]\0\n",
            "a".repeat(40),
            "b".repeat(40)
        );

        assert_eq!(
            parse_marked_stashes(input.as_bytes(), marker).unwrap(),
            vec![identity('b', "stash@{1}")]
        );
    }

    #[test]
    fn exact_store_followed_by_snapshot_failure_is_a_structured_partial_without_reset() {
        let oid = "e".repeat(40);
        let executor = PartialCreateExecutor {
            oid: oid.clone(),
            stored: Mutex::new(Vec::new()),
            snapshots: Mutex::new(VecDeque::from([Err(StashActionError::InvalidOutput)])),
            status_calls: AtomicUsize::new(0),
            drift_on_second_status: false,
            reset_called: Mutex::new(false),
        };

        let status = repository_status(&executor, Path::new("/repo")).unwrap();
        let expected = RepositoryStatePrecondition {
            expected_head: status.branch.oid.clone(),
            expected_head_name: status.branch.head.clone(),
            expected_detached: status.branch.detached,
            expected_unborn: status.branch.unborn,
            expected_index_fingerprint: status.index_fingerprint,
            expected_worktree_fingerprint: status.worktree_fingerprint,
        };
        let result = push_tracked_exact(&executor, Path::new("/repo"), "snapshot", &[], &expected)
            .expect("post-mutation failures are structured");

        assert_eq!(result.state, StashPushState::Partial);
        assert_eq!(result.mutation_oid.as_deref(), Some(oid.as_str()));
        assert!(result.stash.is_none());
        assert!(result.mutation_may_have_occurred);
        assert!(result.error_message.unwrap().contains("was not reset"));
        assert_eq!(executor.stored.lock().unwrap().as_slice(), &[oid]);
        assert!(!*executor.reset_called.lock().unwrap());
    }

    #[test]
    fn tracked_creation_refuses_reset_when_worktree_drifts_after_create() {
        let oid = "e".repeat(40);
        let executor = PartialCreateExecutor {
            oid: oid.clone(),
            stored: Mutex::new(Vec::new()),
            snapshots: Mutex::new(VecDeque::from([Ok(vec![StashIdentity {
                oid: oid.clone(),
                selector: "stash@{0}".to_owned(),
            }])])),
            status_calls: AtomicUsize::new(0),
            drift_on_second_status: true,
            reset_called: Mutex::new(false),
        };
        let status = repository_status(&executor, Path::new("/repo")).unwrap();
        let expected = RepositoryStatePrecondition {
            expected_head: status.branch.oid.clone(),
            expected_head_name: status.branch.head.clone(),
            expected_detached: status.branch.detached,
            expected_unborn: status.branch.unborn,
            expected_index_fingerprint: status.index_fingerprint,
            expected_worktree_fingerprint: status.worktree_fingerprint,
        };

        let result =
            push_tracked_exact(&executor, Path::new("/repo"), "snapshot", &[], &expected).unwrap();

        assert_eq!(result.state, StashPushState::Partial);
        assert!(result.error_message.unwrap().contains("was not reset"));
        assert!(!*executor.reset_called.lock().unwrap());
    }

    #[test]
    fn drop_race_recovers_a_foreign_oid_instead_of_silently_accepting_the_deletion() {
        let target = identity('a', "stash@{0}");
        let foreign = identity('b', "stash@{1}");
        let external = identity('c', "stash@{0}");
        let executor = RacingDropExecutor {
            snapshots: Mutex::new(VecDeque::from([
                Ok(vec![target.clone(), foreign.clone()]),
                Ok(vec![
                    identity('b', "stash@{0}"),
                    identity('c', "stash@{1}"),
                    identity('a', "stash@{2}"),
                ]),
            ])),
            recovered: Mutex::new(Vec::new()),
            dropped_oid: external.oid.clone(),
        };

        let outcome = drop_exact(&executor, Path::new("/repo"), &target.oid);

        assert_eq!(outcome.state, StashCleanupState::Failed);
        assert!(outcome.mutation_may_have_occurred);
        assert!(outcome.error.unwrap().contains("recovery was attempted"));
        assert_eq!(
            executor.recovered.lock().unwrap().as_slice(),
            &[external.oid]
        );
    }

    #[test]
    fn drop_reconciles_the_exact_target_even_when_a_foreign_stash_is_added() {
        let target = identity('a', "stash@{0}");
        let foreign = identity('b', "stash@{1}");
        let executor = RacingDropExecutor {
            snapshots: Mutex::new(VecDeque::from([
                Ok(vec![target.clone(), foreign.clone()]),
                Ok(vec![identity('c', "stash@{0}"), identity('b', "stash@{1}")]),
            ])),
            recovered: Mutex::new(Vec::new()),
            dropped_oid: target.oid.clone(),
        };

        let outcome = drop_exact(&executor, Path::new("/repo"), &target.oid);

        assert_eq!(outcome.state, StashCleanupState::Dropped);
        assert!(
            outcome
                .error
                .unwrap()
                .contains("concurrent stash additions")
        );
        assert!(executor.recovered.lock().unwrap().is_empty());
    }

    #[test]
    fn drop_never_reports_success_when_a_selector_race_leaves_the_target_present() {
        let target = identity('a', "stash@{0}");
        let foreign = identity('b', "stash@{1}");
        let executor = RacingDropExecutor {
            snapshots: Mutex::new(VecDeque::from([
                Ok(vec![target.clone(), foreign.clone()]),
                Ok(vec![
                    identity('c', "stash@{0}"),
                    identity('a', "stash@{1}"),
                    identity('b', "stash@{2}"),
                ]),
            ])),
            recovered: Mutex::new(Vec::new()),
            dropped_oid: identity('c', "stash@{0}").oid,
        };

        let outcome = drop_exact(&executor, Path::new("/repo"), &target.oid);

        assert_eq!(outcome.state, StashCleanupState::Failed);
        assert!(outcome.mutation_may_have_occurred);
        assert!(outcome.error.unwrap().contains("dropped foreign stash"));
    }

    #[test]
    fn post_drop_snapshot_failure_is_structured_as_mutation_uncertainty() {
        let target = identity('a', "stash@{0}");
        let executor = RacingDropExecutor {
            snapshots: Mutex::new(VecDeque::from([
                Ok(vec![target.clone()]),
                Err(StashActionError::InvalidOutput),
            ])),
            recovered: Mutex::new(Vec::new()),
            dropped_oid: target.oid.clone(),
        };

        let outcome = drop_exact(&executor, Path::new("/repo"), &target.oid);

        assert_eq!(outcome.state, StashCleanupState::Failed);
        assert!(outcome.mutation_may_have_occurred);
        assert!(outcome.error.unwrap().contains("reconciliation failed"));
    }
}
