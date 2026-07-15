use std::{ffi::OsString, path::Path, time::Duration};

use app_domain::FileDiff;
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::RepositoryRuntime;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const PATCH_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const LITERAL_PATHSPEC_PREFIX: &str = ":(literal)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiffQuery {
    oid: String,
    path: String,
    old_path: Option<String>,
}

impl FileDiffQuery {
    fn new(oid: &str, path: &str, old_path: Option<&str>) -> Result<Self, FileDiffRuntimeError> {
        if !valid_oid(oid) {
            return Err(FileDiffRuntimeError::InvalidObjectId);
        }
        if !valid_path(path) || old_path.is_some_and(|path| !valid_path(path)) {
            return Err(FileDiffRuntimeError::InvalidPath);
        }
        Ok(Self {
            oid: oid.to_owned(),
            path: path.to_owned(),
            old_path: old_path
                .filter(|old_path| *old_path != path)
                .map(str::to_owned),
        })
    }

    fn arguments(&self) -> Vec<OsString> {
        let mut arguments = [
            "show".to_owned(),
            "--format=".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-textconv".to_owned(),
            "--no-color".to_owned(),
            "-M".to_owned(),
            "--unified=80".to_owned(),
            self.oid.clone(),
            "--".to_owned(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        if let Some(old_path) = &self.old_path {
            arguments.push(OsString::from(format!(
                "{LITERAL_PATHSPEC_PREFIX}{old_path}"
            )));
        }
        arguments.push(OsString::from(format!(
            "{LITERAL_PATHSPEC_PREFIX}{}",
            self.path
        )));
        arguments
    }
}

pub trait FileDiffGitExecutor: Send + Sync {
    fn execute_file_diff(
        &self,
        repository: &Path,
        query: FileDiffQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl FileDiffGitExecutor for GitRunner {
    fn execute_file_diff(
        &self,
        repository: &Path,
        query: FileDiffQuery,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(PATCH_OUTPUT_LIMIT, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

impl<E: FileDiffGitExecutor> RepositoryRuntime<E> {
    pub fn file_diff(
        &self,
        repository: &Path,
        oid: &str,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<FileDiff, FileDiffRuntimeError> {
        file_diff(&self.executor, repository, oid, path, old_path)
    }
}

#[derive(Debug, Error)]
pub enum FileDiffRuntimeError {
    #[error("invalid commit object id")]
    InvalidObjectId,
    #[error("file path must be non-empty and losslessly representable as UTF-8")]
    InvalidPath,
    #[error(transparent)]
    Git(#[from] GitRunError),
}

pub fn file_diff<E: FileDiffGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<FileDiff, FileDiffRuntimeError> {
    let query = FileDiffQuery::new(oid, path, old_path)?;
    let output = executor.execute_file_diff(repository, query)?;
    let patch = String::from_utf8_lossy(&output.stdout).into_owned();
    let binary = is_binary_patch(&patch);
    Ok(FileDiff {
        oid: oid.to_owned(),
        path: path.to_owned(),
        patch,
        binary,
        truncated: false,
    })
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty() && !path.contains(['\0', '\u{fffd}'])
}

fn is_binary_patch(patch: &str) -> bool {
    patch.lines().any(|line| {
        line == "GIT binary patch"
            || (line.starts_with("Binary files ") && line.ends_with(" differ"))
            || line.starts_with("Binary file ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command, sync::Mutex};
    const OID: &str = "0123456789012345678901234567890123456789";

    struct RecordingExecutor {
        queries: Mutex<Vec<FileDiffQuery>>,
        output: Vec<u8>,
    }
    impl FileDiffGitExecutor for RecordingExecutor {
        fn execute_file_diff(
            &self,
            _repository: &Path,
            query: FileDiffQuery,
        ) -> Result<GitOutput, GitRunError> {
            self.queries.lock().unwrap().push(query);
            Ok(GitOutput {
                stdout: self.output.clone(),
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn query_has_one_exact_safe_argv_shape_and_a_literal_pathspec() {
        let query = FileDiffQuery::new(OID, "--output=/tmp/pwn :(glob)*.rs", None).unwrap();
        let arguments = query
            .arguments()
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "show",
                "--format=",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=80",
                OID,
                "--",
                ":(literal)--output=/tmp/pwn :(glob)*.rs"
            ]
        );
    }

    #[test]
    fn rejects_invalid_and_oversized_identifiers_and_invalid_paths_before_git() {
        let executor = RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            output: Vec::new(),
        };
        let invalid_oids = [
            String::new(),
            "abc".to_owned(),
            "a".repeat(41),
            "a".repeat(65),
            "z".repeat(40),
        ];
        for oid in &invalid_oids {
            assert!(matches!(
                file_diff(&executor, Path::new("/repo"), oid, "file.txt", None),
                Err(FileDiffRuntimeError::InvalidObjectId)
            ));
        }
        for path in ["", "unsafe\0path", "lossy\u{fffd}path"] {
            assert!(matches!(
                file_diff(&executor, Path::new("/repo"), OID, path, None),
                Err(FileDiffRuntimeError::InvalidPath)
            ));
        }
        assert!(executor.queries.lock().unwrap().is_empty());
    }

    #[test]
    fn decodes_for_display_and_marks_binary_without_claiming_truncation() {
        let executor = RecordingExecutor { queries: Mutex::new(Vec::new()), output: b"diff --git a/image.bin b/image.bin\nBinary files a/image.bin and b/image.bin differ\n\xff".to_vec() };
        let result = file_diff(&executor, Path::new("/repo"), OID, "image.bin", None).unwrap();
        assert!(result.binary);
        assert!(!result.truncated);
        assert!(result.patch.ends_with('\u{fffd}'));
        assert_eq!(result.path, "image.bin");
    }

    #[test]
    fn real_git_returns_a_rename_patch_when_selected_by_new_path() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        git(directory.path(), &["config", "user.name", "Diff Test"]);
        git(
            directory.path(),
            &["config", "user.email", "diff@example.test"],
        );
        git(directory.path(), &["config", "color.ui", "always"]);
        git(directory.path(), &["config", "diff.renames", "false"]);
        let original = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        fs::write(directory.path().join("old name.txt"), original).unwrap();
        git(directory.path(), &["add", "old name.txt"]);
        git(directory.path(), &["commit", "-m", "base"]);
        git(directory.path(), &["mv", "old name.txt", "new name.txt"]);
        fs::write(
            directory.path().join("new name.txt"),
            format!("{original}after\n"),
        )
        .unwrap();
        git(directory.path(), &["commit", "-am", "rename"]);
        let oid = git_output(directory.path(), &["rev-parse", "HEAD"]);
        let result = RepositoryRuntime::default()
            .file_diff(
                directory.path(),
                oid.trim(),
                "new name.txt",
                Some("old name.txt"),
            )
            .unwrap();
        assert!(result.patch.contains("diff --git"));
        assert!(!result.patch.contains('\u{1b}'));
        assert!(result.patch.contains("rename from old name.txt"));
        assert!(result.patch.contains("rename to new name.txt"));
        assert!(result.patch.contains("new name.txt"));
        assert!(result.patch.contains("+after"));
    }

    #[test]
    fn real_git_marks_binary_content() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        git(directory.path(), &["config", "user.name", "Diff Test"]);
        git(
            directory.path(),
            &["config", "user.email", "diff@example.test"],
        );
        fs::write(directory.path().join("asset.bin"), [0, 1, 2, 3]).unwrap();
        git(directory.path(), &["add", "asset.bin"]);
        git(directory.path(), &["commit", "-m", "binary"]);
        let oid = git_output(directory.path(), &["rev-parse", "HEAD"]);

        let result = RepositoryRuntime::default()
            .file_diff(directory.path(), oid.trim(), "asset.bin", None)
            .unwrap();

        assert!(result.binary);
        assert!(result.patch.contains("Binary files"));
        assert!(!result.truncated);
    }

    fn git(repository: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn git_output(repository: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }
}
