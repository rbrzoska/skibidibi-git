mod branch_actions;
mod branch_creation;
mod conflict_resolution;
mod file_diff;
mod history;
mod maintenance;
mod mutations;
mod navigation;
mod network_operations;
mod stash_actions;
mod stash_inspection;
mod working_tree_diff;

pub use branch_actions::{BranchActionGitExecutor, BranchSwitchError};
pub use branch_creation::{BranchCreationError, BranchCreationGitExecutor};
pub use conflict_resolution::{ConflictResolutionError, ConflictResolutionGitExecutor};
pub use file_diff::{FileDiffGitExecutor, FileDiffQuery, FileDiffRuntimeError, file_diff};
pub use history::{
    DEFAULT_HISTORY_PAGE_SIZE, HistoryGitExecutor, HistoryQuery, HistoryRuntimeError,
    MAX_HISTORY_PAGE_SIZE, commit_details, history_page,
};
pub use maintenance::{MaintenanceError, MaintenanceGitExecutor};
pub use mutations::{MutationGitExecutor, MutationRuntimeError};
pub use navigation::{NavigationGitExecutor, NavigationQuery, NavigationRuntimeError};
pub use network_operations::{NetworkGitExecutor, NetworkOperationError, UpstreamBinding};
pub use stash_actions::{StashActionError, StashActionGitExecutor};
pub use stash_inspection::{
    StashInspectionError, StashInspectionGitExecutor, StashInspectionQuery, stash_details,
    stash_file_diff,
};
pub use working_tree_diff::{
    WorkingTreeDiffGitExecutor, WorkingTreeDiffQuery, WorkingTreeDiffRuntimeError,
    working_tree_file_diff,
};

use std::path::{Component, Path};
use std::time::{Duration, UNIX_EPOCH};

use app_domain::RepositoryStatus;
use git_core::{
    GitExecutor, GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner,
    RemoteMetadataError, RepositoryMetadata, StatusParseError, discover_repository_metadata,
    parse_porcelain_v2_z,
};
use thiserror::Error;

const STATUS_ARGUMENTS: &[&str] = &[
    "status",
    "--porcelain=v2",
    "--branch",
    "-z",
    "--untracked-files=all",
];
const INDEX_ARGUMENTS: &[&str] = &["ls-files", "--stage", "-z"];
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const STATUS_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const STATUS_STDERR_LIMIT: usize = 256 * 1024;

pub trait RepositoryStatusGitExecutor: Send + Sync {
    fn execute_repository_status(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn execute_index_entries(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
}

impl RepositoryStatusGitExecutor for GitRunner {
    fn execute_repository_status(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, STATUS_ARGUMENTS)
                .with_output_limits(STATUS_OUTPUT_LIMIT, STATUS_STDERR_LIMIT)
                .with_timeout(STATUS_TIMEOUT),
        )
    }

    fn execute_index_entries(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, INDEX_ARGUMENTS)
                .with_output_limits(STATUS_OUTPUT_LIMIT, STATUS_STDERR_LIMIT)
                .with_timeout(STATUS_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum RepositoryRuntimeError {
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    InvalidStatus(#[from] StatusParseError),
    #[error(transparent)]
    InvalidMetadata(#[from] RemoteMetadataError),
}

#[derive(Debug, Clone)]
pub struct RepositoryRuntime<E = GitRunner> {
    executor: E,
}

impl Default for RepositoryRuntime<GitRunner> {
    fn default() -> Self {
        Self::new(GitRunner::default())
    }
}

impl<E> RepositoryRuntime<E> {
    pub fn new(executor: E) -> Self {
        Self { executor }
    }
}

impl<E> RepositoryRuntime<E>
where
    E: RepositoryStatusGitExecutor,
{
    pub fn status(&self, repository: &Path) -> Result<RepositoryStatus, RepositoryRuntimeError> {
        repository_status(&self.executor, repository)
    }
}

impl<E> RepositoryRuntime<E>
where
    E: GitExecutor,
{
    pub fn metadata(
        &self,
        repository: &Path,
    ) -> Result<RepositoryMetadata, RepositoryRuntimeError> {
        Ok(discover_repository_metadata(&self.executor, repository)?)
    }
}

pub fn repository_status<E: RepositoryStatusGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Result<RepositoryStatus, RepositoryRuntimeError> {
    let status_output = executor.execute_repository_status(repository)?;
    let index_output = executor.execute_index_entries(repository)?;
    let mut status = parse_porcelain_v2_z(&status_output.stdout)?;
    status.index_fingerprint = index_fingerprint(&index_output.stdout);
    status.worktree_fingerprint =
        worktree_fingerprint(repository, &status_output.stdout, &status.entries);
    Ok(status)
}

pub fn index_fingerprint(index_entries: &[u8]) -> String {
    fingerprint("index", index_entries)
}

pub fn worktree_fingerprint(
    repository: &Path,
    status_output: &[u8],
    entries: &[app_domain::StatusEntry],
) -> String {
    let mut snapshot = Vec::from(status_output);
    for entry in entries {
        snapshot.extend_from_slice(entry.path.as_bytes());
        snapshot.push(0);
        let relative = Path::new(&entry.path);
        if !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            snapshot.extend_from_slice(b"unsafe-path\0");
            continue;
        }
        match std::fs::symlink_metadata(repository.join(relative)) {
            Ok(metadata) => {
                snapshot.extend_from_slice(&metadata.len().to_le_bytes());
                let kind = if metadata.file_type().is_symlink() {
                    2_u8
                } else if metadata.is_file() {
                    1_u8
                } else if metadata.is_dir() {
                    3_u8
                } else {
                    4_u8
                };
                snapshot.push(kind);
                match metadata.modified().and_then(|modified| {
                    modified
                        .duration_since(UNIX_EPOCH)
                        .map_err(std::io::Error::other)
                }) {
                    Ok(modified) => snapshot.extend_from_slice(&modified.as_nanos().to_le_bytes()),
                    Err(_) => snapshot.extend_from_slice(&0_u128.to_le_bytes()),
                }
            }
            Err(error) => {
                snapshot.push(0);
                snapshot.extend_from_slice(format!("{:?}", error.kind()).as_bytes());
            }
        }
    }
    fingerprint("worktree", &snapshot)
}

fn fingerprint(kind: &str, bytes: &[u8]) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let hash = bytes.iter().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
    });
    format!("{kind}-v1:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, path::Path, sync::Mutex};

    use git_core::GitOutput;

    use super::*;

    struct RecordingExecutor {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        output: Vec<u8>,
        index_output: Vec<u8>,
    }

    struct SequenceExecutor {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        outputs: Mutex<VecDeque<Vec<u8>>>,
    }

    impl GitExecutor for SequenceExecutor {
        fn execute(&self, repository: &Path, arguments: &[&str]) -> Result<GitOutput, GitRunError> {
            self.calls.lock().expect("calls lock").push((
                repository.display().to_string(),
                arguments.iter().map(|value| (*value).to_owned()).collect(),
            ));
            Ok(GitOutput {
                stdout: self
                    .outputs
                    .lock()
                    .expect("outputs lock")
                    .pop_front()
                    .expect("recorded output"),
                stderr: Vec::new(),
            })
        }
    }

    impl GitExecutor for RecordingExecutor {
        fn execute(&self, repository: &Path, arguments: &[&str]) -> Result<GitOutput, GitRunError> {
            self.calls.lock().expect("calls lock").push((
                repository.display().to_string(),
                arguments.iter().map(|value| (*value).to_owned()).collect(),
            ));
            Ok(GitOutput {
                stdout: self.output.clone(),
                stderr: Vec::new(),
            })
        }
    }

    impl RepositoryStatusGitExecutor for RecordingExecutor {
        fn execute_repository_status(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
            self.execute(repository, STATUS_ARGUMENTS)
        }

        fn execute_index_entries(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
            self.calls.lock().expect("calls lock").push((
                repository.display().to_string(),
                INDEX_ARGUMENTS
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            ));
            Ok(GitOutput {
                stdout: self.index_output.clone(),
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn status_uses_the_fixed_porcelain_v2_nul_delimited_command() {
        let executor = RecordingExecutor {
            calls: Mutex::new(Vec::new()),
            output: b"# branch.oid (initial)\0# branch.head main\0".to_vec(),
            index_output: Vec::new(),
        };
        let runtime = RepositoryRuntime::new(executor);

        let status = runtime
            .status(Path::new("/tmp/repository;echo unsafe"))
            .expect("status succeeds");

        assert!(status.branch.unborn);
        let calls = runtime.executor.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "/tmp/repository;echo unsafe");
        assert_eq!(calls[0].1, STATUS_ARGUMENTS);
        assert_eq!(calls[1].1, INDEX_ARGUMENTS);
        assert_eq!(status.index_fingerprint, index_fingerprint(&[]));
    }

    #[test]
    fn metadata_uses_fixed_argv_and_keeps_remote_name_as_one_argument() {
        let executor = SequenceExecutor {
            calls: Mutex::new(Vec::new()),
            outputs: Mutex::new(VecDeque::from([
                b"feature/safe\n".to_vec(),
                b"upstream;echo unsafe\n".to_vec(),
                b"origin\nupstream;echo unsafe\n".to_vec(),
                b"git@github.com:owner/repository.git\n".to_vec(),
                b"ssh://git@github.com/owner/repository.git\n".to_vec(),
            ])),
        };
        let runtime = RepositoryRuntime::new(executor);

        let metadata = runtime
            .metadata(Path::new("/tmp/repository;echo unsafe"))
            .expect("metadata succeeds");

        assert_eq!(
            metadata.default_remote.as_deref(),
            Some("upstream;echo unsafe")
        );
        assert_eq!(metadata.provider, app_domain::RepositoryProvider::GitHub);
        let calls = runtime.executor.calls.lock().expect("calls lock");
        assert_eq!(
            calls
                .iter()
                .map(|(_, arguments)| arguments.clone())
                .collect::<Vec<_>>(),
            [
                vec!["symbolic-ref", "--quiet", "--short", "HEAD"],
                vec!["config", "--get", "branch.feature/safe.remote"],
                vec!["remote"],
                vec!["remote", "get-url", "--", "upstream;echo unsafe"],
                vec!["remote", "get-url", "--push", "--", "upstream;echo unsafe"],
            ]
        );
        assert!(
            calls
                .iter()
                .all(|(repository, _)| repository == "/tmp/repository;echo unsafe")
        );
    }
}
