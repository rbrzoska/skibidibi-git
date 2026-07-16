use std::{ffi::OsString, path::Path, time::Duration};

use app_domain::{
    AutoStashCreateState, AutoStashOutcome, MergeBranchRequest, MergeBranchResult,
    MergeBranchState, PopStashRequest, PullInactiveBranchRequest, PullInactiveBranchResult,
    PushStashRequest, RepositoryStatePrecondition, RepositoryStatus, StashCleanupState,
    StashPushState, StashRestoreState, WorktreeDirtyState,
};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, NavigationParseError,
    StatusParseError, parse_porcelain_v2_z, parse_worktree_porcelain_z,
};
use thiserror::Error;

use crate::{
    RepositoryRuntime, RepositoryRuntimeError, StashActionError, StashActionGitExecutor,
    repository_status,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const BINDING_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(symref)%00";
const WORKTREE_STATUS_ARGS: &[&str] = &[
    "status",
    "--porcelain=v2",
    "--branch",
    "-z",
    "--untracked-files=all",
];

pub trait BranchOperationGitExecutor: Send + Sync {
    fn query_branch_binding(
        &self,
        repository: &Path,
        full_name: &str,
    ) -> Result<GitOutput, GitRunError>;
    fn merge_exact_oid(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
    fn query_merge_origin(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn query_worktrees_for_operations(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn fetch_exact_upstream(
        &self,
        repository: &Path,
        remote: &str,
        remote_ref: &str,
        upstream_ref: &str,
    ) -> Result<GitOutput, GitRunError>;
    fn is_ancestor(
        &self,
        repository: &Path,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, GitRunError>;
    fn update_branch_ref(
        &self,
        repository: &Path,
        full_name: &str,
        new_oid: &str,
        expected_old_oid: &str,
    ) -> Result<GitOutput, GitRunError>;
    fn fast_forward_worktree(&self, worktree: &Path, oid: &str) -> Result<GitOutput, GitRunError>;
    fn worktree_status(&self, worktree: &Path) -> Result<GitOutput, GitRunError>;
}

impl BranchOperationGitExecutor for GitRunner {
    fn query_branch_binding(
        &self,
        repository: &Path,
        full_name: &str,
    ) -> Result<GitOutput, GitRunError> {
        let filter = if full_name.starts_with("refs/remotes/") {
            "refs/remotes"
        } else {
            full_name
        };
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["for-each-ref", BINDING_FORMAT, filter],
            )
            .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn merge_exact_oid(&self, repository: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["merge", "--no-edit", "--", oid],
            )
            .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn query_merge_origin(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["rev-parse", "--verify", "ORIG_HEAD^{commit}"],
            )
            .with_output_limits(1024, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn query_worktrees_for_operations(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["worktree", "list", "--porcelain", "-z"],
            )
            .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn fetch_exact_upstream(
        &self,
        repository: &Path,
        remote: &str,
        remote_ref: &str,
        upstream_ref: &str,
    ) -> Result<GitOutput, GitRunError> {
        let refspec = format!("+{remote_ref}:{upstream_ref}");
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Network,
                [
                    OsString::from("-c"),
                    OsString::from("credential.interactive=never"),
                    OsString::from("fetch"),
                    OsString::from("--no-write-fetch-head"),
                    OsString::from(remote),
                    OsString::from(refspec),
                ],
            )
            .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(NETWORK_TIMEOUT),
        )
    }

    fn is_ancestor(
        &self,
        repository: &Path,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, GitRunError> {
        match self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["merge-base", "--is-ancestor", ancestor, descendant],
            )
            .with_output_limits(1024, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        ) {
            Ok(_) => Ok(true),
            Err(GitRunError::Unsuccessful { code: Some(1), .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn update_branch_ref(
        &self,
        repository: &Path,
        full_name: &str,
        new_oid: &str,
        expected_old_oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["update-ref", full_name, new_oid, expected_old_oid],
            )
            .with_output_limits(1024, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn fast_forward_worktree(&self, worktree: &Path, oid: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            worktree,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["merge", "--ff-only", "--", oid],
            )
            .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn worktree_status(&self, worktree: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            worktree,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, WORKTREE_STATUS_ARGS)
                .with_output_limits(OUTPUT_LIMIT, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum BranchOperationError {
    #[error("the branch operation request is invalid")]
    InvalidRequest,
    #[error("the requested branch does not exist or changed; refresh before retrying")]
    StaleBranch,
    #[error("the selected source is the current target branch")]
    SameBranch,
    #[error("the target must be the currently checked-out local branch")]
    TargetNotCurrent,
    #[error("the selected branch is checked out in a worktree")]
    BranchCheckedOut,
    #[error("the branch has no configured upstream or it changed")]
    UpstreamChanged,
    #[error("the remote update is not a fast-forward")]
    NonFastForward,
    #[error("the working tree is dirty; enable auto-stash to continue")]
    DirtyWorkingTree,
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Stash(#[from] StashActionError),
    #[error(transparent)]
    Worktrees(#[from] NavigationParseError),
    #[error(transparent)]
    Status(#[from] StatusParseError),
    #[error("Git returned malformed branch metadata")]
    InvalidOutput,
}

impl BranchOperationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest | Self::InvalidOutput => "invalidRequest",
            Self::StaleBranch | Self::UpstreamChanged => "staleState",
            Self::SameBranch => "sameBranch",
            Self::TargetNotCurrent => "targetNotCurrent",
            Self::BranchCheckedOut => "branchCheckedOut",
            Self::NonFastForward => "nonFastForward",
            Self::DirtyWorkingTree => "dirtyWorkingTree",
            Self::Stash(error) => error.code(),
            Self::Git(_) | Self::Repository(_) | Self::Worktrees(_) | Self::Status(_) => {
                "gitRejected"
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Binding {
    full_name: String,
    oid: String,
    current: bool,
    upstream: Option<String>,
    upstream_ref: Option<String>,
    remote: Option<String>,
    remote_ref: Option<String>,
    symbolic_target: Option<String>,
}

impl<E> RepositoryRuntime<E>
where
    E: BranchOperationGitExecutor + StashActionGitExecutor,
{
    pub fn merge_branch(
        &self,
        repository: &Path,
        request: &MergeBranchRequest,
    ) -> Result<MergeBranchResult, BranchOperationError> {
        validate_merge_request(request)?;
        let before = repository_status(&self.executor, repository)?;
        let source = binding(&self.executor, repository, &request.source_full_name)?;
        let target = binding(&self.executor, repository, &request.target_full_name)?;
        if source.oid != request.expected_source_oid || target.oid != request.expected_target_oid {
            return Err(BranchOperationError::StaleBranch);
        }
        if source.symbolic_target.is_some() || target.symbolic_target.is_some() {
            return Err(BranchOperationError::InvalidRequest);
        }
        if !target.current
            || before.branch.head.as_deref() != request.target_full_name.strip_prefix("refs/heads/")
            || before.branch.oid.as_deref() != Some(request.expected_target_oid.as_str())
        {
            return Err(BranchOperationError::TargetNotCurrent);
        }
        let mut auto_stash = AutoStashOutcome::not_requested();
        if !before.entries.is_empty() {
            let options = request
                .auto_stash
                .as_ref()
                .ok_or(BranchOperationError::DirtyWorkingTree)?;
            let pushed = self.push_stash(
                repository,
                &PushStashRequest {
                    message: options.message.clone(),
                    include_untracked: true,
                    precondition: precondition(&before),
                },
            )?;
            auto_stash = auto_stash_from_push(&pushed);
            if !matches!(
                pushed.state,
                StashPushState::Created | StashPushState::NoChanges
            ) {
                return Ok(operation_failure(
                    request.expected_target_oid.clone(),
                    pushed.status,
                    auto_stash,
                    pushed.error_message,
                ));
            }
        } else if request.auto_stash.is_some() {
            auto_stash.create = AutoStashCreateState::NotNeeded;
        }

        let rebound_source = binding(&self.executor, repository, &request.source_full_name)?;
        let rebound_target = binding(&self.executor, repository, &request.target_full_name)?;
        if rebound_source != source || rebound_target != target {
            restore_stash(self, repository, &mut auto_stash);
            return Err(BranchOperationError::StaleBranch);
        }

        let operation = self.executor.merge_exact_oid(repository, &source.oid);
        let mut status = repository_status(&self.executor, repository).ok();
        if status.is_none() {
            retain_stash(&mut auto_stash, StashRestoreState::SkippedUnsafe);
            return Ok(MergeBranchResult {
                state: MergeBranchState::Failed,
                head_before: request.expected_target_oid.clone(),
                head_after: None,
                status: None,
                auto_stash,
                error_message: Some(
                    "repository status could not be verified after merge; refresh before recovery"
                        .to_owned(),
                ),
                mutation_may_have_occurred: true,
            });
        }
        let observed_head = status
            .as_ref()
            .and_then(|value| value.branch.oid.as_deref());
        if observed_head.is_some_and(|head| head != request.expected_target_oid) {
            let origin_matches = self
                .executor
                .query_merge_origin(repository)
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .is_some_and(|origin| origin.trim() == request.expected_target_oid);
            if !origin_matches {
                retain_stash(&mut auto_stash, StashRestoreState::SkippedUnsafe);
                return Ok(MergeBranchResult {
                    state: MergeBranchState::Failed,
                    head_before: request.expected_target_oid.clone(),
                    head_after: status.as_ref().and_then(|value| value.branch.oid.clone()),
                    status,
                    auto_stash,
                    error_message: Some(
                        "the target branch changed during merge; refresh before recovery"
                            .to_owned(),
                    ),
                    mutation_may_have_occurred: true,
                });
            }
        }
        let conflicted = status.as_ref().is_some_and(has_conflicts);
        if conflicted {
            retain_stash(&mut auto_stash, StashRestoreState::SkippedUnsafe);
        } else {
            restore_stash(self, repository, &mut auto_stash);
            status = repository_status(&self.executor, repository)
                .ok()
                .or(status);
        }
        let state = if conflicted || status.as_ref().is_some_and(has_conflicts) {
            MergeBranchState::Conflicted
        } else if operation.is_err() || stash_incomplete(&auto_stash) {
            MergeBranchState::Failed
        } else {
            MergeBranchState::Succeeded
        };
        Ok(MergeBranchResult {
            state,
            head_before: request.expected_target_oid.clone(),
            head_after: status.as_ref().and_then(|value| value.branch.oid.clone()),
            status,
            auto_stash,
            error_message: operation.err().map(|error| error.to_string()),
            mutation_may_have_occurred: false,
        })
    }

    pub fn pull_inactive_branch(
        &self,
        repository: &Path,
        request: &PullInactiveBranchRequest,
    ) -> Result<PullInactiveBranchResult, BranchOperationError> {
        validate_local_ref(&request.branch_full_name)?;
        validate_oid(&request.expected_oid)?;
        let initial = binding(&self.executor, repository, &request.branch_full_name)?;
        if initial.symbolic_target.is_some() {
            return Err(BranchOperationError::InvalidRequest);
        }
        if initial.current || initial.oid != request.expected_oid {
            return Err(if initial.current {
                BranchOperationError::BranchCheckedOut
            } else {
                BranchOperationError::StaleBranch
            });
        }
        if initial.upstream.as_deref() != Some(request.expected_upstream.as_str()) {
            return Err(BranchOperationError::UpstreamChanged);
        }
        let checked_out =
            checked_out_worktree(&self.executor, repository, &request.branch_full_name)?;
        if let Some(path) = checked_out.as_deref() {
            ensure_clean_worktree(&self.executor, path)?;
        }
        let remote = initial
            .remote
            .as_deref()
            .ok_or(BranchOperationError::UpstreamChanged)?;
        let remote_ref = initial
            .remote_ref
            .as_deref()
            .ok_or(BranchOperationError::UpstreamChanged)?;
        let upstream_ref = initial
            .upstream_ref
            .as_deref()
            .ok_or(BranchOperationError::UpstreamChanged)?;
        self.executor
            .fetch_exact_upstream(repository, remote, remote_ref, upstream_ref)?;

        let rebound = binding(&self.executor, repository, &request.branch_full_name)?;
        if rebound.full_name != initial.full_name
            || rebound.oid != initial.oid
            || rebound.current
            || rebound.upstream != initial.upstream
            || rebound.upstream_ref != initial.upstream_ref
            || rebound.remote != initial.remote
            || rebound.remote_ref != initial.remote_ref
        {
            return Err(BranchOperationError::StaleBranch);
        }
        let rebound_worktree =
            checked_out_worktree(&self.executor, repository, &request.branch_full_name)?;
        if rebound_worktree != checked_out {
            return Err(BranchOperationError::StaleBranch);
        }
        if let Some(path) = rebound_worktree.as_deref() {
            ensure_clean_worktree(&self.executor, path)?;
        }
        let upstream = binding(&self.executor, repository, upstream_ref)?;
        if !self
            .executor
            .is_ancestor(repository, &initial.oid, &upstream.oid)?
        {
            return Err(BranchOperationError::NonFastForward);
        }
        if upstream.oid != initial.oid {
            if let Some(worktree) = rebound_worktree.as_deref() {
                self.executor
                    .fast_forward_worktree(worktree, &upstream.oid)?;
            } else {
                self.executor.update_branch_ref(
                    repository,
                    &initial.full_name,
                    &upstream.oid,
                    &initial.oid,
                )?;
            }
        }
        let final_binding = binding(&self.executor, repository, &request.branch_full_name)?;
        if final_binding.oid != upstream.oid {
            return Err(BranchOperationError::StaleBranch);
        }
        Ok(PullInactiveBranchResult {
            branch_full_name: initial.full_name,
            head_before: initial.oid.clone(),
            head_after: upstream.oid.clone(),
            upstream: upstream_ref.to_owned(),
            changed: initial.oid != upstream.oid,
        })
    }

    pub fn worktree_dirty_states(
        &self,
        repository: &Path,
    ) -> Result<Vec<WorktreeDirtyState>, BranchOperationError> {
        let output = self.executor.query_worktrees_for_operations(repository)?;
        let worktrees = parse_worktree_porcelain_z(&output.stdout)?;
        Ok(worktrees
            .into_iter()
            .filter(|worktree| !worktree.bare && !worktree.prunable)
            .map(|worktree| {
                let path = worktree.path.clone();
                match std::fs::canonicalize(&path) {
                    Ok(canonical) if canonical.is_dir() => match self
                        .executor
                        .worktree_status(&canonical)
                    {
                        Ok(output) => match parse_porcelain_v2_z(&output.stdout) {
                            Ok(status) => WorktreeDirtyState {
                                branch_full_name: worktree.branch,
                                worktree_path: path,
                                dirty: !status.entries.is_empty(),
                                change_count: status.entries.len(),
                                error_message: None,
                            },
                            Err(error) => {
                                dirty_state_error(worktree.branch, path, error.to_string())
                            }
                        },
                        Err(error) => dirty_state_error(worktree.branch, path, error.to_string()),
                    },
                    _ => dirty_state_error(
                        worktree.branch,
                        path,
                        "worktree path is unavailable".to_owned(),
                    ),
                }
            })
            .collect())
    }
}

fn validate_merge_request(request: &MergeBranchRequest) -> Result<(), BranchOperationError> {
    validate_ref(&request.source_full_name)?;
    validate_local_ref(&request.target_full_name)?;
    validate_oid(&request.expected_source_oid)?;
    validate_oid(&request.expected_target_oid)?;
    if request.source_full_name == request.target_full_name {
        return Err(BranchOperationError::SameBranch);
    }
    Ok(())
}

fn validate_ref(value: &str) -> Result<(), BranchOperationError> {
    let name = value
        .strip_prefix("refs/heads/")
        .or_else(|| value.strip_prefix("refs/remotes/"));
    if value.len() <= 1024 && name.is_some_and(valid_branch_name) {
        Ok(())
    } else {
        Err(BranchOperationError::InvalidRequest)
    }
}

fn valid_branch_name(name: &str) -> bool {
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

fn validate_local_ref(value: &str) -> Result<(), BranchOperationError> {
    validate_ref(value)?;
    if value.starts_with("refs/heads/") {
        Ok(())
    } else {
        Err(BranchOperationError::InvalidRequest)
    }
}

fn validate_oid(value: &str) -> Result<(), BranchOperationError> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(BranchOperationError::InvalidRequest)
    }
}

fn binding<E: BranchOperationGitExecutor>(
    executor: &E,
    repository: &Path,
    full_name: &str,
) -> Result<Binding, BranchOperationError> {
    validate_ref(full_name)?;
    let output = executor.query_branch_binding(repository, full_name)?;
    for line in output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        if let Some(binding) = parse_binding_line(line, full_name)? {
            return Ok(binding);
        }
    }
    Err(BranchOperationError::StaleBranch)
}

fn parse_binding_line(
    input: &[u8],
    full_name: &str,
) -> Result<Option<Binding>, BranchOperationError> {
    let fields = input.split(|byte| *byte == 0).collect::<Vec<_>>();
    let [
        found_ref,
        oid,
        current,
        upstream,
        upstream_ref,
        remote,
        remote_ref,
        symbolic_target,
        trailing,
    ] = fields.as_slice()
    else {
        return Err(BranchOperationError::InvalidOutput);
    };
    if !trailing.is_empty() {
        return Err(BranchOperationError::InvalidOutput);
    }
    let found_ref = decode_field(found_ref)?;
    if found_ref != full_name {
        return Ok(None);
    }
    let oid = decode_field(oid)?;
    validate_oid(oid)?;
    let optional = |field: &[u8]| -> Result<Option<String>, BranchOperationError> {
        (!field.is_empty())
            .then(|| decode_field(field).map(str::to_owned))
            .transpose()
    };
    let binding = Binding {
        full_name: found_ref.to_owned(),
        oid: oid.to_owned(),
        current: decode_field(current)? == "*",
        upstream: optional(upstream)?,
        upstream_ref: optional(upstream_ref)?,
        remote: optional(remote)?,
        remote_ref: optional(remote_ref)?,
        symbolic_target: optional(symbolic_target)?,
    };
    let upstream_complete = binding.upstream.is_some() == binding.upstream_ref.is_some()
        && binding.upstream.is_some() == binding.remote.is_some()
        && binding.upstream.is_some() == binding.remote_ref.is_some();
    let valid_upstream = binding.upstream.as_deref().is_none_or(valid_branch_name)
        && binding.upstream_ref.as_deref().is_none_or(|value| {
            value
                .strip_prefix("refs/remotes/")
                .is_some_and(valid_branch_name)
        })
        && binding.remote.as_deref().is_none_or(|value| {
            !value.is_empty() && !value.starts_with('-') && !value.chars().any(char::is_control)
        })
        && binding.remote_ref.as_deref().is_none_or(|value| {
            value
                .strip_prefix("refs/heads/")
                .is_some_and(valid_branch_name)
        });
    if !upstream_complete || !valid_upstream {
        return Err(BranchOperationError::InvalidOutput);
    }
    Ok(Some(binding))
}

fn decode_field(field: &[u8]) -> Result<&str, BranchOperationError> {
    std::str::from_utf8(field).map_err(|_| BranchOperationError::InvalidOutput)
}

fn checked_out_worktree<E: BranchOperationGitExecutor>(
    executor: &E,
    repository: &Path,
    branch: &str,
) -> Result<Option<std::path::PathBuf>, BranchOperationError> {
    let output = executor.query_worktrees_for_operations(repository)?;
    let found = parse_worktree_porcelain_z(&output.stdout)?
        .into_iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch));
    found
        .map(|worktree| {
            if worktree.bare || worktree.prunable || worktree.locked {
                return Err(BranchOperationError::BranchCheckedOut);
            }
            std::fs::canonicalize(worktree.path).map_err(|_| BranchOperationError::BranchCheckedOut)
        })
        .transpose()
}

fn ensure_clean_worktree<E: BranchOperationGitExecutor>(
    executor: &E,
    worktree: &Path,
) -> Result<(), BranchOperationError> {
    let output = executor.worktree_status(worktree)?;
    let status = parse_porcelain_v2_z(&output.stdout)?;
    if status.entries.is_empty() {
        Ok(())
    } else {
        Err(BranchOperationError::DirtyWorkingTree)
    }
}

fn precondition(status: &RepositoryStatus) -> RepositoryStatePrecondition {
    RepositoryStatePrecondition {
        expected_head: status.branch.oid.clone(),
        expected_head_name: status.branch.head.clone(),
        expected_detached: status.branch.detached,
        expected_unborn: status.branch.unborn,
        expected_index_fingerprint: status.index_fingerprint.clone(),
        expected_worktree_fingerprint: status.worktree_fingerprint.clone(),
    }
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

fn restore_stash<E>(
    runtime: &RepositoryRuntime<E>,
    repository: &Path,
    outcome: &mut AutoStashOutcome,
) where
    E: BranchOperationGitExecutor + StashActionGitExecutor,
{
    let Some(stash) = outcome.stash.clone() else {
        return;
    };
    let status = match repository_status(&runtime.executor, repository) {
        Ok(status) => status,
        Err(error) => {
            outcome.restore = StashRestoreState::Failed;
            outcome.restore_error = Some(error.to_string());
            return;
        }
    };
    match runtime.pop_stash(
        repository,
        &PopStashRequest {
            stash,
            restore_index: true,
            precondition: precondition(&status),
        },
    ) {
        Ok(result) => {
            outcome.restore = result.restore;
            outcome.cleanup = result.cleanup;
            outcome.restore_error = result.restore_error;
            outcome.cleanup_error = result.cleanup_error;
        }
        Err(error) => {
            outcome.restore = StashRestoreState::Failed;
            outcome.cleanup = StashCleanupState::Retained;
            outcome.restore_error = Some(error.to_string());
        }
    }
}

fn retain_stash(outcome: &mut AutoStashOutcome, restore: StashRestoreState) {
    if outcome.stash.is_some() {
        outcome.restore = restore;
        outcome.cleanup = StashCleanupState::Retained;
    }
}

fn has_conflicts(status: &RepositoryStatus) -> bool {
    status
        .entries
        .iter()
        .any(|entry| entry.kind == app_domain::StatusEntryKind::Unmerged)
}

fn stash_incomplete(outcome: &AutoStashOutcome) -> bool {
    matches!(
        outcome.create,
        AutoStashCreateState::Failed | AutoStashCreateState::Partial
    ) || matches!(
        outcome.restore,
        StashRestoreState::Failed
            | StashRestoreState::Conflicted
            | StashRestoreState::SkippedUnsafe
    )
}

fn operation_failure(
    head_before: String,
    status: Option<RepositoryStatus>,
    auto_stash: AutoStashOutcome,
    error_message: Option<String>,
) -> MergeBranchResult {
    MergeBranchResult {
        state: MergeBranchState::Failed,
        head_before,
        head_after: status.as_ref().and_then(|value| value.branch.oid.clone()),
        status,
        auto_stash,
        error_message,
        mutation_may_have_occurred: false,
    }
}

fn dirty_state_error(
    branch_full_name: Option<String>,
    worktree_path: String,
    error_message: String,
) -> WorktreeDirtyState {
    WorktreeDirtyState {
        branch_full_name,
        worktree_path,
        dirty: false,
        change_count: 0,
        error_message: Some(error_message),
    }
}
