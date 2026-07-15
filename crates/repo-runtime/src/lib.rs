mod branch_actions;
mod file_diff;
mod history;
mod navigation;

pub use branch_actions::{BranchActionGitExecutor, BranchSwitchError};
pub use file_diff::{FileDiffGitExecutor, FileDiffQuery, FileDiffRuntimeError, file_diff};
pub use history::{
    DEFAULT_HISTORY_PAGE_SIZE, HistoryGitExecutor, HistoryQuery, HistoryRuntimeError,
    MAX_HISTORY_PAGE_SIZE, commit_details, history_page,
};
pub use navigation::{NavigationGitExecutor, NavigationQuery, NavigationRuntimeError};

use std::path::Path;

use app_domain::RepositoryStatus;
use git_core::{
    GitExecutor, GitRunError, GitRunner, RemoteMetadataError, RepositoryMetadata, StatusParseError,
    discover_repository_metadata, parse_porcelain_v2_z,
};
use thiserror::Error;

const STATUS_ARGUMENTS: &[&str] = &[
    "status",
    "--porcelain=v2",
    "--branch",
    "-z",
    "--untracked-files=all",
];

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

impl<E> RepositoryRuntime<E>
where
    E: GitExecutor,
{
    pub fn new(executor: E) -> Self {
        Self { executor }
    }

    pub fn status(&self, repository: &Path) -> Result<RepositoryStatus, RepositoryRuntimeError> {
        let output = self.executor.execute(repository, STATUS_ARGUMENTS)?;
        Ok(parse_porcelain_v2_z(&output.stdout)?)
    }

    pub fn metadata(
        &self,
        repository: &Path,
    ) -> Result<RepositoryMetadata, RepositoryRuntimeError> {
        Ok(discover_repository_metadata(&self.executor, repository)?)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, path::Path, sync::Mutex};

    use git_core::GitOutput;

    use super::*;

    struct RecordingExecutor {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        output: Vec<u8>,
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

    #[test]
    fn status_uses_the_fixed_porcelain_v2_nul_delimited_command() {
        let executor = RecordingExecutor {
            calls: Mutex::new(Vec::new()),
            output: b"# branch.oid (initial)\0# branch.head main\0".to_vec(),
        };
        let runtime = RepositoryRuntime::new(executor);

        let status = runtime
            .status(Path::new("/tmp/repository;echo unsafe"))
            .expect("status succeeds");

        assert!(status.branch.unborn);
        let calls = runtime.executor.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "/tmp/repository;echo unsafe");
        assert_eq!(calls[0].1, STATUS_ARGUMENTS);
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
