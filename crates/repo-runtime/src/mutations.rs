use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use app_domain::{
    AmendCommitRequest, AmendCommitResult, AmendCommitState, ApplyIndexChangeRequest,
    ApplyIndexChangeResult, ChangeSelection, CreateCommitRequest, CreateCommitResult, IndexAction,
    RepositoryStatus, StatusCode, StatusEntry, StatusEntryKind, WorkingTreeEntrySelector,
};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{
    RepositoryRuntime, RepositoryRuntimeError, RepositoryStatusGitExecutor, repository_status,
};

const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const COMMIT_TIMEOUT: Duration = Duration::from_secs(120);
const OUTPUT_LIMIT: usize = 512 * 1024;
const MAX_SELECTED_ENTRIES: usize = 10_000;
const MAX_TOTAL_PATH_BYTES: usize = 1024 * 1024;
const MAX_COMMIT_MESSAGE_BYTES: usize = 1024 * 1024;
const LITERAL_PATHSPEC_PREFIX: &str = ":(literal)";

pub trait MutationGitExecutor: RepositoryStatusGitExecutor {
    fn stage(
        &self,
        repository: &Path,
        all: bool,
        pathspecs: Vec<String>,
    ) -> Result<GitOutput, GitRunError>;

    fn unstage(
        &self,
        repository: &Path,
        all: bool,
        unborn: bool,
        pathspecs: Vec<String>,
    ) -> Result<GitOutput, GitRunError>;

    fn create_commit(&self, repository: &Path, message: &[u8]) -> Result<GitOutput, GitRunError>;

    fn amend_commit(
        &self,
        repository: &Path,
        message: Option<&[u8]>,
    ) -> Result<GitOutput, GitRunError>;

    fn commit_parents(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
}

impl MutationGitExecutor for GitRunner {
    fn stage(
        &self,
        repository: &Path,
        all: bool,
        pathspecs: Vec<String>,
    ) -> Result<GitOutput, GitRunError> {
        let mut invocation = GitInvocation::new(GitInvocationPolicy::Mutating, ["add", "--all"]);
        if !all {
            invocation = GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    "add",
                    "--all",
                    "--pathspec-from-file=-",
                    "--pathspec-file-nul",
                ],
            )
            .with_stdin(encode_pathspecs(&pathspecs));
        }
        self.run(
            repository,
            invocation
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn unstage(
        &self,
        repository: &Path,
        all: bool,
        unborn: bool,
        pathspecs: Vec<String>,
    ) -> Result<GitOutput, GitRunError> {
        let invocation = if unborn && all {
            GitInvocation::new(GitInvocationPolicy::Mutating, ["read-tree", "--empty"])
        } else if unborn {
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [
                    "rm",
                    "--cached",
                    "--ignore-unmatch",
                    "--pathspec-from-file=-",
                    "--pathspec-file-nul",
                ],
            )
            .with_stdin(encode_pathspecs(&pathspecs))
        } else {
            if all {
                GitInvocation::new(
                    GitInvocationPolicy::Mutating,
                    ["restore", "--staged", "--source=HEAD", "--", "."],
                )
            } else {
                GitInvocation::new(
                    GitInvocationPolicy::Mutating,
                    [
                        "restore",
                        "--staged",
                        "--source=HEAD",
                        "--pathspec-from-file=-",
                        "--pathspec-file-nul",
                    ],
                )
                .with_stdin(encode_pathspecs(&pathspecs))
            }
        };
        self.run(
            repository,
            invocation
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn create_commit(&self, repository: &Path, message: &[u8]) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["commit", "--file=-", "--cleanup=strip"],
            )
            .with_stdin(message.to_vec())
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(COMMIT_TIMEOUT),
        )
    }

    fn amend_commit(
        &self,
        repository: &Path,
        message: Option<&[u8]>,
    ) -> Result<GitOutput, GitRunError> {
        let invocation = match message {
            Some(message) => GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["commit", "--amend", "--file=-", "--cleanup=strip"],
            )
            .with_stdin(message.to_vec()),
            None => GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["commit", "--amend", "--no-edit"],
            ),
        };
        self.run(
            repository,
            invocation
                .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
                .with_timeout(COMMIT_TIMEOUT),
        )
    }

    fn commit_parents(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["show", "-s", "--format=%P", oid, "--"],
            )
            .with_output_limits(OUTPUT_LIMIT, OUTPUT_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum MutationRuntimeError {
    #[error("the mutation request is invalid")]
    InvalidRequest,
    #[error("a selected path is not a safe repository-relative path")]
    InvalidPath,
    #[error("repository HEAD or index changed; refresh before retrying")]
    StaleState,
    #[error("a selected change is no longer present; refresh before retrying")]
    ChangeNotFound,
    #[error("conflicted entries require the conflict resolver")]
    ConflictsPresent,
    #[error("the selected entries cannot be {action}")]
    IneligibleSelection { action: &'static str },
    #[error("there are no staged changes to commit")]
    NothingStaged,
    #[error("there is no existing commit to amend")]
    NothingToAmend,
    #[error(
        "the current commit is reachable from the configured upstream; confirm the rewrite before retrying"
    )]
    UpstreamRewriteConfirmationRequired,
    #[error("the commit message must contain non-whitespace text and be at most {limit} bytes")]
    InvalidCommitMessage { limit: usize },
    #[error("Git reported success but did not return the new commit id")]
    MissingCommitId,
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
}

impl MutationRuntimeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::InvalidPath | Self::InvalidCommitMessage { .. } => {
                "invalidRequest"
            }
            Self::StaleState | Self::ChangeNotFound => "staleState",
            Self::ConflictsPresent => "conflictsPresent",
            Self::IneligibleSelection { .. } => "ineligibleSelection",
            Self::NothingStaged => "nothingStaged",
            Self::NothingToAmend => "nothingToAmend",
            Self::UpstreamRewriteConfirmationRequired => "upstreamRewriteConfirmationRequired",
            Self::MissingCommitId | Self::Repository(_) => "internal",
            Self::Git(GitRunError::TimedOut { .. }) => "timedOut",
            Self::Git(GitRunError::OutputLimitExceeded { .. }) => "outputLimit",
            Self::Git(_) => "gitRejected",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::StaleState | Self::ChangeNotFound | Self::Git(GitRunError::TimedOut { .. })
        )
    }
}

impl<E: MutationGitExecutor> RepositoryRuntime<E> {
    pub fn apply_index_change(
        &self,
        repository: &Path,
        request: &ApplyIndexChangeRequest,
    ) -> Result<ApplyIndexChangeResult, MutationRuntimeError> {
        let before = repository_status(&self.executor, repository)?;
        validate_precondition(
            &before,
            request.expected_head.as_deref(),
            request.expected_head_name.as_deref(),
            request.expected_detached,
            request.expected_unborn,
            &request.expected_index_fingerprint,
            &request.expected_worktree_fingerprint,
        )?;
        reject_conflicts(&before)?;

        let (all, pathspecs, changed) = selection_for_action(&before, request)?;
        if !changed {
            return Ok(ApplyIndexChangeResult {
                changed: false,
                status: before,
            });
        }

        match request.action {
            IndexAction::Stage => {
                self.executor.stage(repository, all, pathspecs)?;
            }
            IndexAction::Unstage => {
                self.executor
                    .unstage(repository, all, before.branch.unborn, pathspecs)?;
            }
        }

        Ok(ApplyIndexChangeResult {
            changed: true,
            status: repository_status(&self.executor, repository)?,
        })
    }

    pub fn create_commit(
        &self,
        repository: &Path,
        request: &CreateCommitRequest,
    ) -> Result<CreateCommitResult, MutationRuntimeError> {
        validate_commit_message(&request.message)?;
        let before = repository_status(&self.executor, repository)?;
        validate_precondition(
            &before,
            request.expected_head.as_deref(),
            request.expected_head_name.as_deref(),
            request.expected_detached,
            request.expected_unborn,
            &request.expected_index_fingerprint,
            &request.expected_worktree_fingerprint,
        )?;
        reject_conflicts(&before)?;
        if !before.entries.iter().any(is_staged) {
            return Err(MutationRuntimeError::NothingStaged);
        }

        self.executor
            .create_commit(repository, request.message.as_bytes())?;
        let status = repository_status(&self.executor, repository)?;
        let oid = status
            .branch
            .oid
            .clone()
            .ok_or(MutationRuntimeError::MissingCommitId)?;
        Ok(CreateCommitResult { oid, status })
    }

    pub fn amend_commit(
        &self,
        repository: &Path,
        request: &AmendCommitRequest,
    ) -> Result<AmendCommitResult, MutationRuntimeError> {
        if let Some(message) = &request.message {
            validate_commit_message(message)?;
        }
        let before = repository_status(&self.executor, repository)?;
        validate_amend_preflight(&before, request)?;
        let previous_oid = before
            .branch
            .oid
            .clone()
            .filter(|_| !before.branch.unborn)
            .ok_or(MutationRuntimeError::NothingToAmend)?;
        let immediately_before = repository_status(&self.executor, repository)?;
        validate_amend_preflight(&immediately_before, request)?;
        let parents_before = self
            .executor
            .commit_parents(repository, &previous_oid)?
            .stdout;

        let amend = self
            .executor
            .amend_commit(repository, request.message.as_deref().map(str::as_bytes));
        let status = match repository_status(&self.executor, repository) {
            Ok(status) => status,
            Err(error) => {
                return Ok(AmendCommitResult {
                    previous_oid,
                    oid: None,
                    status: None,
                    state: AmendCommitState::OutcomeUnknown,
                    error_message: Some(format!(
                        "the amend outcome could not be inspected; refresh and inspect HEAD and the index before retrying: {error}"
                    )),
                });
            }
        };

        if let Err(error) = amend {
            if same_repository_state(&status, &immediately_before) {
                return Err(MutationRuntimeError::Git(error));
            }
            return Ok(amend_outcome_unknown(
                previous_oid,
                status,
                format!(
                    "Git rejected the amend after repository state changed; refresh and inspect HEAD and the index before retrying: {error}"
                ),
            ));
        }

        let Some(oid) = status.branch.oid.clone() else {
            return Ok(amend_outcome_unknown(
                previous_oid,
                status,
                "Git reported a successful amend but HEAD has no commit; refresh and inspect the repository before retrying".to_owned(),
            ));
        };
        let branch_location_matches = status.branch.head == immediately_before.branch.head
            && status.branch.detached == immediately_before.branch.detached
            && !status.branch.unborn;
        let parents_match = match self.executor.commit_parents(repository, &oid) {
            Ok(parents) => parents.stdout == parents_before,
            Err(error) => {
                return Ok(amend_outcome_unknown(
                    previous_oid,
                    status,
                    format!(
                        "the amended commit could not be verified; refresh and inspect HEAD before retrying: {error}"
                    ),
                ));
            }
        };
        if !branch_location_matches || !parents_match {
            return Ok(amend_outcome_unknown(
                previous_oid,
                status,
                "the amended HEAD did not satisfy the expected postcondition; refresh and inspect repository history before retrying".to_owned(),
            ));
        }

        Ok(AmendCommitResult {
            previous_oid,
            oid: Some(oid),
            status: Some(status),
            state: AmendCommitState::Succeeded,
            error_message: None,
        })
    }
}

fn validate_amend_preflight(
    status: &RepositoryStatus,
    request: &AmendCommitRequest,
) -> Result<(), MutationRuntimeError> {
    validate_precondition(
        status,
        request.expected_head.as_deref(),
        request.expected_head_name.as_deref(),
        request.expected_detached,
        request.expected_unborn,
        &request.expected_index_fingerprint,
        &request.expected_worktree_fingerprint,
    )?;
    reject_conflicts(status)?;
    if status.branch.upstream.is_some()
        && status.branch.ahead == 0
        && !request.confirm_upstream_rewrite
    {
        return Err(MutationRuntimeError::UpstreamRewriteConfirmationRequired);
    }
    Ok(())
}

fn same_repository_state(left: &RepositoryStatus, right: &RepositoryStatus) -> bool {
    left.branch == right.branch
        && left.index_fingerprint == right.index_fingerprint
        && left.worktree_fingerprint == right.worktree_fingerprint
}

fn amend_outcome_unknown(
    previous_oid: String,
    status: RepositoryStatus,
    error_message: String,
) -> AmendCommitResult {
    AmendCommitResult {
        previous_oid,
        oid: status.branch.oid.clone(),
        status: Some(status),
        state: AmendCommitState::OutcomeUnknown,
        error_message: Some(error_message),
    }
}

fn validate_precondition(
    status: &RepositoryStatus,
    expected_head: Option<&str>,
    expected_head_name: Option<&str>,
    expected_detached: bool,
    expected_unborn: bool,
    expected_index_fingerprint: &str,
    expected_worktree_fingerprint: &str,
) -> Result<(), MutationRuntimeError> {
    if expected_index_fingerprint.is_empty()
        || expected_worktree_fingerprint.is_empty()
        || status.branch.oid.as_deref() != expected_head
        || status.branch.head.as_deref() != expected_head_name
        || status.branch.detached != expected_detached
        || status.branch.unborn != expected_unborn
        || status.index_fingerprint != expected_index_fingerprint
        || status.worktree_fingerprint != expected_worktree_fingerprint
    {
        return Err(MutationRuntimeError::StaleState);
    }
    Ok(())
}

fn reject_conflicts(status: &RepositoryStatus) -> Result<(), MutationRuntimeError> {
    if status
        .entries
        .iter()
        .any(|entry| entry.kind == StatusEntryKind::Unmerged)
    {
        return Err(MutationRuntimeError::ConflictsPresent);
    }
    Ok(())
}

fn selection_for_action(
    status: &RepositoryStatus,
    request: &ApplyIndexChangeRequest,
) -> Result<(bool, Vec<String>, bool), MutationRuntimeError> {
    match &request.selection {
        ChangeSelection::All => Ok((
            true,
            Vec::new(),
            status
                .entries
                .iter()
                .any(|entry| eligible(entry, request.action)),
        )),
        ChangeSelection::Selected { entries } => {
            if entries.is_empty() || entries.len() > MAX_SELECTED_ENTRIES {
                return Err(MutationRuntimeError::InvalidRequest);
            }
            let total_bytes = entries.iter().try_fold(0_usize, |total, selector| {
                total.checked_add(selector.path.len()).and_then(|value| {
                    selector
                        .old_path
                        .as_ref()
                        .map_or(Some(value), |path| value.checked_add(path.len()))
                })
            });
            if total_bytes.is_none_or(|value| value > MAX_TOTAL_PATH_BYTES) {
                return Err(MutationRuntimeError::InvalidRequest);
            }

            let mut identities = BTreeSet::new();
            let mut paths = BTreeSet::new();
            for selector in entries {
                validate_selector(selector)?;
                let identity = format!(
                    "{:?}\0{}\0{}",
                    selector.entry_kind,
                    selector.path,
                    selector.old_path.as_deref().unwrap_or("")
                );
                if !identities.insert(identity) {
                    return Err(MutationRuntimeError::InvalidRequest);
                }
                let matches = status
                    .entries
                    .iter()
                    .filter(|entry| selector_matches(selector, entry))
                    .collect::<Vec<_>>();
                let [entry] = matches.as_slice() else {
                    return Err(MutationRuntimeError::ChangeNotFound);
                };
                if !eligible(entry, request.action) {
                    return Err(MutationRuntimeError::IneligibleSelection {
                        action: match request.action {
                            IndexAction::Stage => "staged",
                            IndexAction::Unstage => "unstaged",
                        },
                    });
                }
                if let Some(old_path) = &entry.original_path {
                    paths.insert(old_path.clone());
                }
                paths.insert(entry.path.clone());
            }

            Ok((
                false,
                paths
                    .into_iter()
                    .map(|path| format!("{LITERAL_PATHSPEC_PREFIX}{path}"))
                    .collect(),
                true,
            ))
        }
    }
}

fn encode_pathspecs(pathspecs: &[String]) -> Vec<u8> {
    let mut input =
        Vec::with_capacity(pathspecs.iter().map(String::len).sum::<usize>() + pathspecs.len());
    for pathspec in pathspecs {
        input.extend_from_slice(pathspec.as_bytes());
        input.push(0);
    }
    input
}

fn validate_selector(selector: &WorkingTreeEntrySelector) -> Result<(), MutationRuntimeError> {
    validate_relative_path(&selector.path)?;
    if let Some(old_path) = &selector.old_path {
        validate_relative_path(old_path)?;
    }
    if selector.entry_kind == StatusEntryKind::RenamedOrCopied && selector.old_path.is_none()
        || selector.entry_kind != StatusEntryKind::RenamedOrCopied && selector.old_path.is_some()
    {
        return Err(MutationRuntimeError::InvalidRequest);
    }
    if matches!(
        selector.entry_kind,
        StatusEntryKind::Ignored | StatusEntryKind::Unmerged
    ) {
        return Err(MutationRuntimeError::InvalidRequest);
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<PathBuf, MutationRuntimeError> {
    if path.is_empty() || path.contains(['\0', '\u{fffd}']) {
        return Err(MutationRuntimeError::InvalidPath);
    }
    let value = Path::new(path);
    if value.is_absolute()
        || !value
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(MutationRuntimeError::InvalidPath);
    }
    Ok(value.to_path_buf())
}

fn selector_matches(selector: &WorkingTreeEntrySelector, entry: &StatusEntry) -> bool {
    entry.path == selector.path
        && entry.original_path == selector.old_path
        && entry.kind == selector.entry_kind
}

fn eligible(entry: &StatusEntry, action: IndexAction) -> bool {
    match action {
        IndexAction::Stage => is_unstaged(entry),
        IndexAction::Unstage => is_staged(entry),
    }
}

fn is_staged(entry: &StatusEntry) -> bool {
    entry.kind != StatusEntryKind::Untracked
        && !matches!(
            entry.index_status,
            StatusCode::Unmodified | StatusCode::Ignored
        )
}

fn is_unstaged(entry: &StatusEntry) -> bool {
    entry.kind == StatusEntryKind::Untracked
        || !matches!(
            entry.worktree_status,
            StatusCode::Unmodified | StatusCode::Ignored
        )
}

fn validate_commit_message(message: &str) -> Result<(), MutationRuntimeError> {
    if message.trim().is_empty()
        || message.len() > MAX_COMMIT_MESSAGE_BYTES
        || message.contains('\0')
    {
        return Err(MutationRuntimeError::InvalidCommitMessage {
            limit: MAX_COMMIT_MESSAGE_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    use super::*;

    struct RacingExecutor {
        statuses: Mutex<VecDeque<Vec<u8>>>,
        amend_called: AtomicBool,
    }

    impl RepositoryStatusGitExecutor for RacingExecutor {
        fn execute_repository_status(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: self
                    .statuses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("recorded status"),
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

    impl MutationGitExecutor for RacingExecutor {
        fn stage(
            &self,
            _repository: &Path,
            _all: bool,
            _pathspecs: Vec<String>,
        ) -> Result<GitOutput, GitRunError> {
            unreachable!()
        }

        fn unstage(
            &self,
            _repository: &Path,
            _all: bool,
            _unborn: bool,
            _pathspecs: Vec<String>,
        ) -> Result<GitOutput, GitRunError> {
            unreachable!()
        }

        fn create_commit(
            &self,
            _repository: &Path,
            _message: &[u8],
        ) -> Result<GitOutput, GitRunError> {
            unreachable!()
        }

        fn amend_commit(
            &self,
            _repository: &Path,
            _message: Option<&[u8]>,
        ) -> Result<GitOutput, GitRunError> {
            self.amend_called.store(true, Ordering::SeqCst);
            unreachable!("stale second preflight must prevent amend")
        }

        fn commit_parents(&self, _repository: &Path, _oid: &str) -> Result<GitOutput, GitRunError> {
            unreachable!("stale second preflight must prevent parent query")
        }
    }

    fn branch_status(oid: &str) -> Vec<u8> {
        format!("# branch.oid {oid}\0# branch.head main\0").into_bytes()
    }

    #[test]
    fn second_amend_preflight_catches_a_deterministic_external_head_race() {
        let original = "a".repeat(40);
        let moved = "b".repeat(40);
        let runtime = RepositoryRuntime::new(RacingExecutor {
            statuses: Mutex::new(VecDeque::from([
                branch_status(&original),
                branch_status(&original),
                branch_status(&moved),
            ])),
            amend_called: AtomicBool::new(false),
        });
        let observed = runtime.status(Path::new("/repo")).unwrap();
        let request = AmendCommitRequest {
            message: None,
            confirm_upstream_rewrite: false,
            expected_head: observed.branch.oid.clone(),
            expected_head_name: observed.branch.head.clone(),
            expected_detached: observed.branch.detached,
            expected_unborn: observed.branch.unborn,
            expected_index_fingerprint: observed.index_fingerprint.clone(),
            expected_worktree_fingerprint: observed.worktree_fingerprint.clone(),
        };

        let error = runtime
            .amend_commit(Path::new("/repo"), &request)
            .expect_err("second preflight rejects moved HEAD");

        assert!(matches!(error, MutationRuntimeError::StaleState));
        assert!(!runtime.executor.amend_called.load(Ordering::SeqCst));
    }
}
