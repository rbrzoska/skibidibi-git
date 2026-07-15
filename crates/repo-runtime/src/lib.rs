use std::path::Path;

use app_domain::RepositoryStatus;
use git_core::{GitExecutor, GitRunError, GitRunner, StatusParseError, parse_porcelain_v2_z};
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
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Mutex};

    use git_core::GitOutput;

    use super::*;

    struct RecordingExecutor {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        output: Vec<u8>,
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
}
