use std::{
    ffi::OsString,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use app_domain::{
    DeleteBranchRequest, DeleteBranchResult, FetchRepositoryResult, PushStashRequest,
    RemoveWorktreeRequest, RemoveWorktreeResult, RepositoryBranchKind, RepositoryStatePrecondition,
    StashIdentity, StashPushState, WorktreeRemovalMode,
};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{
    NavigationGitExecutor, NavigationRuntimeError, RepositoryRuntime, RepositoryRuntimeError,
    RepositoryStatusGitExecutor, StashActionError, StashActionGitExecutor,
};

const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const STDOUT_LIMIT: usize = 512 * 1024;
const STDERR_LIMIT: usize = 512 * 1024;

pub trait MaintenanceGitExecutor: Send + Sync {
    fn verify_merged_into_head(
        &self,
        repository: &Path,
        expected_oid: &str,
    ) -> Result<GitOutput, GitRunError>;

    fn remove_worktree(
        &self,
        repository: &Path,
        path: &Path,
        force: bool,
    ) -> Result<GitOutput, GitRunError>;

    fn fetch_default_remote(&self, repository: &Path) -> Result<GitOutput, GitRunError>;

    fn compare_and_delete_ref(
        &self,
        repository: &Path,
        full_name: &str,
        expected_oid: &str,
    ) -> Result<GitOutput, GitRunError>;

    fn restore_deleted_ref(
        &self,
        repository: &Path,
        full_name: &str,
        expected_oid: &str,
    ) -> Result<GitOutput, GitRunError>;
}

impl MaintenanceGitExecutor for GitRunner {
    fn verify_merged_into_head(
        &self,
        repository: &Path,
        expected_oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["merge-base", "--is-ancestor", expected_oid, "HEAD"],
            )
            .with_output_limits(STDOUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn remove_worktree(
        &self,
        repository: &Path,
        path: &Path,
        force: bool,
    ) -> Result<GitOutput, GitRunError> {
        let mut arguments = vec![OsString::from("worktree"), OsString::from("remove")];
        if force {
            arguments.push(OsString::from("--force"));
        }
        arguments.push(OsString::from("--"));
        arguments.push(path.as_os_str().to_owned());
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::Mutating, arguments)
                .with_output_limits(STDOUT_LIMIT, STDERR_LIMIT)
                .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn fetch_default_remote(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
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
                ],
            )
            .with_output_limits(STDOUT_LIMIT, STDERR_LIMIT)
            .with_timeout(FETCH_TIMEOUT),
        )
    }

    fn compare_and_delete_ref(
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
            .with_output_limits(STDOUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }

    fn restore_deleted_ref(
        &self,
        repository: &Path,
        full_name: &str,
        expected_oid: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                vec![
                    OsString::from("update-ref"),
                    OsString::from(full_name),
                    OsString::from(expected_oid),
                    OsString::from("0".repeat(expected_oid.len())),
                ],
            )
            .with_output_limits(STDOUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum MaintenanceError {
    #[error("the maintenance request is invalid")]
    InvalidRequest,
    #[error("the requested branch does not exist or changed")]
    BranchChanged,
    #[error("the current branch cannot be deleted")]
    CurrentBranch,
    #[error("the branch is checked out in a worktree")]
    BranchInUse,
    #[error("the requested worktree does not exist or changed")]
    WorktreeChanged,
    #[error("the current worktree cannot be removed")]
    CurrentWorktree,
    #[error("bare, locked, or prunable worktrees cannot be removed")]
    WorktreeUnavailable,
    #[error(transparent)]
    Navigation(#[from] NavigationRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error("system time is outside the supported range")]
    InvalidClock,
    #[error("worktree changes could not be safely stashed: {0}")]
    StashRejected(String),
    #[error(transparent)]
    Stash(#[from] StashActionError),
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
}

impl MaintenanceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalidRequest",
            Self::BranchChanged | Self::WorktreeChanged => "staleState",
            Self::CurrentBranch | Self::BranchInUse => "branchInUse",
            Self::CurrentWorktree | Self::WorktreeUnavailable => "worktreeUnavailable",
            Self::Navigation(_) | Self::Git(_) => "gitRejected",
            Self::InvalidClock => "internal",
            Self::StashRejected(_) | Self::Stash(_) => "stashRejected",
            Self::Repository(_) => "gitRejected",
        }
    }
}

impl<E> RepositoryRuntime<E>
where
    E: NavigationGitExecutor
        + MaintenanceGitExecutor
        + RepositoryStatusGitExecutor
        + StashActionGitExecutor,
{
    pub fn delete_branch(
        &self,
        repository: &Path,
        request: &DeleteBranchRequest,
    ) -> Result<DeleteBranchResult, MaintenanceError> {
        super::branch_actions::validate_requested_ref(&request.full_name)
            .map_err(|_| MaintenanceError::InvalidRequest)?;
        let branches = self.branches(repository)?;
        let branch = branches
            .iter()
            .find(|branch| branch.full_name == request.full_name)
            .filter(|branch| branch.oid == request.expected_oid)
            .ok_or(MaintenanceError::BranchChanged)?;
        if branch.kind != RepositoryBranchKind::Local || branch.symbolic_target.is_some() {
            return Err(MaintenanceError::InvalidRequest);
        }
        if branch.current {
            return Err(MaintenanceError::CurrentBranch);
        }
        if self
            .worktrees(repository)?
            .iter()
            .any(|worktree| worktree.branch.as_deref() == Some(request.full_name.as_str()))
        {
            return Err(MaintenanceError::BranchInUse);
        }
        self.executor
            .verify_merged_into_head(repository, &request.expected_oid)?;
        self.executor.compare_and_delete_ref(
            repository,
            &request.full_name,
            &request.expected_oid,
        )?;
        match self.worktrees(repository) {
            Ok(worktrees)
                if worktrees.iter().any(|worktree| {
                    worktree.branch.as_deref() == Some(request.full_name.as_str())
                }) =>
            {
                self.executor.restore_deleted_ref(
                    repository,
                    &request.full_name,
                    &request.expected_oid,
                )?;
                return Err(MaintenanceError::BranchInUse);
            }
            Ok(_) => {}
            Err(error) => {
                self.executor.restore_deleted_ref(
                    repository,
                    &request.full_name,
                    &request.expected_oid,
                )?;
                return Err(MaintenanceError::Navigation(error));
            }
        }
        Ok(DeleteBranchResult {
            full_name: request.full_name.clone(),
            deleted: true,
        })
    }

    pub fn remove_worktree_and_branch(
        &self,
        repository: &Path,
        request: &RemoveWorktreeRequest,
    ) -> Result<RemoveWorktreeResult, MaintenanceError> {
        let requested_path = Path::new(&request.path);
        if !requested_path.is_absolute() {
            return Err(MaintenanceError::InvalidRequest);
        }
        let requested_canonical =
            std::fs::canonicalize(requested_path).map_err(|_| MaintenanceError::WorktreeChanged)?;
        let repository_canonical =
            std::fs::canonicalize(repository).map_err(|_| MaintenanceError::WorktreeChanged)?;
        if requested_canonical == repository_canonical {
            return Err(MaintenanceError::CurrentWorktree);
        }
        let worktrees = self.worktrees(repository)?;
        let worktree = worktrees
            .iter()
            .find(|worktree| {
                std::fs::canonicalize(&worktree.path).is_ok_and(|path| path == requested_canonical)
            })
            .filter(|worktree| worktree.head == request.expected_head)
            .filter(|worktree| worktree.branch == request.branch_full_name)
            .ok_or(MaintenanceError::WorktreeChanged)?;
        if worktree.bare || worktree.locked || worktree.prunable {
            return Err(MaintenanceError::WorktreeUnavailable);
        }
        match request.mode {
            WorktreeRemovalMode::Safe | WorktreeRemovalMode::Force
                if request.stash_message.is_some() =>
            {
                return Err(MaintenanceError::InvalidRequest);
            }
            WorktreeRemovalMode::StashAndForce if request.stash_message.is_none() => {
                return Err(MaintenanceError::InvalidRequest);
            }
            _ => {}
        }
        if let Some(branch) = &request.branch_full_name {
            super::branch_actions::validate_requested_ref(branch)
                .map_err(|_| MaintenanceError::InvalidRequest)?;
            let expected_head = worktree
                .head
                .as_deref()
                .ok_or(MaintenanceError::WorktreeChanged)?;
            let branches = self.branches(repository)?;
            let associated = branches
                .iter()
                .find(|candidate| candidate.full_name == *branch)
                .filter(|candidate| candidate.oid == expected_head)
                .filter(|candidate| candidate.kind == RepositoryBranchKind::Local)
                .ok_or(MaintenanceError::BranchChanged)?;
            if associated.current || associated.symbolic_target.is_some() {
                return Err(MaintenanceError::BranchInUse);
            }
        }

        let stash = if request.mode == WorktreeRemovalMode::StashAndForce {
            let target = Path::new(&worktree.path);
            let status = self.status(target)?;
            let stash_result = self.push_stash(
                target,
                &PushStashRequest {
                    message: request
                        .stash_message
                        .clone()
                        .ok_or(MaintenanceError::InvalidRequest)?,
                    include_untracked: true,
                    precondition: precondition_from_status(&status),
                },
            )?;
            let clean = stash_result
                .status
                .as_ref()
                .is_some_and(|status| status.entries.is_empty());
            match stash_result.state {
                StashPushState::NoChanges if clean => None,
                StashPushState::Created if clean => stash_result.stash,
                state => {
                    return Ok(RemoveWorktreeResult {
                        path: request.path.clone(),
                        branch_full_name: request.branch_full_name.clone(),
                        worktree_removed: false,
                        worktree_removal_error: Some(format!(
                            "stash ended in {state:?}: {}",
                            stash_result.error_message.unwrap_or_else(||
                                "the worktree was not verified clean".to_owned()
                            )
                        )),
                        branch_deleted: false,
                        branch_deletion_error: None,
                        mode: request.mode,
                        stash: stash_result.stash,
                    });
                }
            }
        } else {
            None
        };

        let refreshed_worktree = match self.worktrees(repository) {
            Ok(worktrees) => worktrees
                .into_iter()
                .find(|candidate| {
                    std::fs::canonicalize(&candidate.path)
                        .is_ok_and(|path| path == requested_canonical)
                })
                .filter(|candidate| candidate.head == request.expected_head)
                .filter(|candidate| candidate.branch == request.branch_full_name),
            Err(error) => {
                return Ok(failed_worktree_removal(
                    request,
                    stash,
                    format!("could not revalidate the worktree after stashing: {error}"),
                ));
            }
        };
        let Some(refreshed_worktree) = refreshed_worktree else {
            return Ok(failed_worktree_removal(
                request,
                stash,
                "the worktree changed after stashing; it was not removed".to_owned(),
            ));
        };

        // After stashing, deliberately use Git's non-force removal. If an editor or watcher
        // creates a new tracked/untracked change after the stash, Git refuses instead of
        // deleting a change that is not represented by the returned stash identity.
        if let Err(error) = self.executor.remove_worktree(
            repository,
            Path::new(&refreshed_worktree.path),
            request.mode == WorktreeRemovalMode::Force,
        ) {
            return Ok(failed_worktree_removal(request, stash, error.to_string()));
        }
        let branch_deletion = if let Some(full_name) = &request.branch_full_name {
            (|| -> Result<bool, MaintenanceError> {
                let expected_head = request
                    .expected_head
                    .as_deref()
                    .ok_or(MaintenanceError::WorktreeChanged)?;
                let branch = self
                    .branches(repository)?
                    .into_iter()
                    .find(|candidate| candidate.full_name == *full_name)
                    .filter(|candidate| candidate.oid == expected_head)
                    .ok_or(MaintenanceError::BranchChanged)?;
                if branch.current
                    || self
                        .worktrees(repository)?
                        .iter()
                        .any(|other| other.branch.as_deref() == Some(full_name.as_str()))
                {
                    return Err(MaintenanceError::BranchInUse);
                }
                self.executor
                    .compare_and_delete_ref(repository, full_name, expected_head)?;
                match self.worktrees(repository) {
                    Ok(worktrees)
                        if worktrees
                            .iter()
                            .any(|other| other.branch.as_deref() == Some(full_name.as_str())) =>
                    {
                        self.executor
                            .restore_deleted_ref(repository, full_name, expected_head)?;
                        Err(MaintenanceError::BranchInUse)
                    }
                    Ok(_) => Ok(true),
                    Err(error) => {
                        self.executor
                            .restore_deleted_ref(repository, full_name, expected_head)?;
                        Err(MaintenanceError::Navigation(error))
                    }
                }
            })()
        } else {
            Ok(false)
        };
        let (branch_deleted, branch_deletion_error) = match branch_deletion {
            Ok(deleted) => (deleted, None),
            Err(error) => (false, Some(error.to_string())),
        };
        Ok(RemoveWorktreeResult {
            path: request.path.clone(),
            branch_full_name: request.branch_full_name.clone(),
            worktree_removed: true,
            worktree_removal_error: None,
            branch_deleted,
            branch_deletion_error,
            mode: request.mode,
            stash,
        })
    }

    pub fn fetch_repository(
        &self,
        repository: &Path,
    ) -> Result<FetchRepositoryResult, MaintenanceError> {
        self.executor.fetch_default_remote(repository)?;
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| MaintenanceError::InvalidClock)?
            .as_secs();
        Ok(FetchRepositoryResult {
            fetched_at: i64::try_from(seconds).map_err(|_| MaintenanceError::InvalidClock)?,
        })
    }
}

fn failed_worktree_removal(
    request: &RemoveWorktreeRequest,
    stash: Option<StashIdentity>,
    error: String,
) -> RemoveWorktreeResult {
    RemoveWorktreeResult {
        path: request.path.clone(),
        branch_full_name: request.branch_full_name.clone(),
        worktree_removed: false,
        worktree_removal_error: Some(error),
        branch_deleted: false,
        branch_deletion_error: None,
        mode: request.mode,
        stash,
    }
}

fn precondition_from_status(status: &app_domain::RepositoryStatus) -> RepositoryStatePrecondition {
    RepositoryStatePrecondition {
        expected_head: status.branch.oid.clone(),
        expected_head_name: status.branch.head.clone(),
        expected_detached: status.branch.detached,
        expected_unborn: status.branch.unborn,
        expected_index_fingerprint: status.index_fingerprint.clone(),
        expected_worktree_fingerprint: status.worktree_fingerprint.clone(),
    }
}
