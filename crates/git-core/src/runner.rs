use std::{
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum GitRunError {
    #[error("failed to start git: {0}")]
    Spawn(#[source] io::Error),
    #[error("git exited unsuccessfully ({code:?}): {stderr}")]
    Unsuccessful { code: Option<i32>, stderr: String },
}

pub trait GitExecutor: Send + Sync {
    fn execute(&self, repository: &Path, arguments: &[&str]) -> Result<GitOutput, GitRunError>;
}

/// Executes Git directly with an argv vector. No command is ever passed through a shell.
#[derive(Debug, Clone)]
pub struct GitRunner {
    executable: PathBuf,
}

impl Default for GitRunner {
    fn default() -> Self {
        Self::new("git")
    }
}

impl GitRunner {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn execute_os<I, S>(
        &self,
        repository: &Path,
        arguments: I,
    ) -> Result<GitOutput, GitRunError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let repository_argument = OsString::from(repository.as_os_str());
        let output = Command::new(&self.executable)
            .arg("--no-pager")
            .arg("-c")
            .arg("core.fsmonitor=false")
            .arg("-C")
            .arg(repository_argument)
            .args(arguments)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(GitRunError::Spawn)?;

        if !output.status.success() {
            return Err(GitRunError::Unsuccessful {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        Ok(GitOutput {
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

impl GitExecutor for GitRunner {
    fn execute(&self, repository: &Path, arguments: &[&str]) -> Result<GitOutput, GitRunError> {
        self.execute_os(repository, arguments)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn repository_path_is_one_argv_value_even_with_shell_metacharacters() {
        let root = tempdir().expect("temporary directory");
        let repository = root.path().join("repo;touch should-not-exist");
        fs::create_dir(&repository).expect("repository directory");
        let init = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repository)
            .status()
            .expect("git init");
        assert!(init.success());

        let output = GitRunner::default()
            .execute(&repository, &["rev-parse", "--is-inside-work-tree"])
            .expect("git command succeeds");

        assert_eq!(output.stdout, b"true\n");
        assert!(!root.path().join("should-not-exist").exists());
    }

    #[test]
    fn unsuccessful_exit_includes_stderr() {
        let directory = tempdir().expect("temporary directory");
        let error = GitRunner::default()
            .execute(directory.path(), &["rev-parse", "--is-inside-work-tree"])
            .expect_err("not a repository");

        match error {
            GitRunError::Unsuccessful { code, stderr } => {
                assert_ne!(code, Some(0));
                assert!(stderr.contains("not a git repository"));
            }
            GitRunError::Spawn(error) => panic!("unexpected spawn failure: {error}"),
        }
    }
}
