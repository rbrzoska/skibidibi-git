use std::{path::Path, time::Duration};

use app_domain::RepositoryStatus;
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{RepositoryRuntimeError, RepositoryStatusGitExecutor, repository_status};

const FILE_ARGUMENTS: &[&str] = &["diff", "--cached", "--name-only", "-z", "--"];
const DIFF_ARGUMENTS: &[&str] = &[
    "diff",
    "--cached",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--unified=3",
    "--",
];
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_FILES: usize = 200;
const MAX_CONTEXT_BYTES: usize = 512 * 1024;
const MAX_GIT_OUTPUT: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedAiContext {
    pub status: RepositoryStatus,
    pub text: String,
}

pub trait AiContextGitExecutor: RepositoryStatusGitExecutor {
    fn staged_file_names(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn staged_diff(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
}

impl AiContextGitExecutor for GitRunner {
    fn staged_file_names(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, FILE_ARGUMENTS)
                .with_output_limits(MAX_GIT_OUTPUT, 64 * 1024)
                .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn staged_diff(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, DIFF_ARGUMENTS)
                .with_output_limits(MAX_GIT_OUTPUT, 64 * 1024)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum AiContextError {
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error("there are no staged changes to describe")]
    Empty,
    #[error("the staged change contains more than {MAX_FILES} files")]
    TooManyFiles,
    #[error("Git returned an invalid staged file list")]
    InvalidFileList,
    #[error("repository state changed while preparing the staged AI context")]
    Stale,
}

pub fn staged_ai_context<E: AiContextGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Result<StagedAiContext, AiContextError> {
    let status = repository_status(executor, repository)?;
    let files = executor.staged_file_names(repository)?.stdout;
    if files.is_empty() {
        return Err(AiContextError::Empty);
    }
    let file_names = files
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| std::str::from_utf8(value).map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AiContextError::InvalidFileList)?;
    if file_names.len() > MAX_FILES {
        return Err(AiContextError::TooManyFiles);
    }

    let sensitive_files = file_names
        .iter()
        .filter(|path| is_sensitive_path(path))
        .cloned()
        .collect::<Vec<_>>();
    let mut diff = if sensitive_files.is_empty() {
        executor.staged_diff(repository)?.stdout
    } else {
        Vec::new()
    };
    let truncated = diff.len() > MAX_CONTEXT_BYTES;
    diff.truncate(MAX_CONTEXT_BYTES);
    while std::str::from_utf8(&diff).is_err() {
        diff.pop();
    }
    let mut text = format!(
        "Staged files ({}):\n{}\n\nStaged diff:\n{}",
        file_names.len(),
        file_names.join("\n"),
        String::from_utf8(diff).expect("truncated to UTF-8 boundary")
    );
    if !sensitive_files.is_empty() {
        text.push_str("\n\nSensitive staged paths omitted from the diff:\n");
        text.push_str(&sensitive_files.join("\n"));
    }
    if truncated {
        text.push_str("\n\n[Diff truncated at 512 KiB]");
    }

    let after = repository_status(executor, repository)?;
    if status.branch.oid != after.branch.oid
        || status.index_fingerprint != after.index_fingerprint
        || status.worktree_fingerprint != after.worktree_fingerprint
    {
        return Err(AiContextError::Stale);
    }

    Ok(StagedAiContext { status, text })
}

fn is_sensitive_path(path: &str) -> bool {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            name.as_str(),
            ".npmrc" | ".pypirc" | ".netrc" | "id_rsa" | "id_ed25519"
        )
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|extension| name.ends_with(extension))
}

pub fn staged_ai_context_default(repository: &Path) -> Result<StagedAiContext, AiContextError> {
    staged_ai_context(&GitRunner::default(), repository)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fake {
        files: Vec<u8>,
        diff: Vec<u8>,
    }

    impl RepositoryStatusGitExecutor for Fake {
        fn execute_repository_status(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout:
                    b"# branch.oid 0123456789012345678901234567890123456789\0# branch.head main\0"
                        .to_vec(),
                stderr: vec![],
            })
        }
        fn execute_index_entries(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: b"index".to_vec(),
                stderr: vec![],
            })
        }
    }

    impl AiContextGitExecutor for Fake {
        fn staged_file_names(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: self.files.clone(),
                stderr: vec![],
            })
        }
        fn staged_diff(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: self.diff.clone(),
                stderr: vec![],
            })
        }
    }

    #[test]
    fn builds_a_bounded_staged_only_context() {
        let fake = Fake {
            files: b"src/a.rs\0".to_vec(),
            diff: vec![b'x'; MAX_CONTEXT_BYTES + 20],
        };
        let context = staged_ai_context(&fake, Path::new("/fixture")).unwrap();
        assert!(context.text.contains("src/a.rs"));
        assert!(context.text.ends_with("[Diff truncated at 512 KiB]"));
        assert!(!context.text.contains("unstaged"));
    }

    #[test]
    fn rejects_empty_and_excessive_file_sets() {
        assert!(matches!(
            staged_ai_context(&Fake::default(), Path::new("/fixture")),
            Err(AiContextError::Empty)
        ));
        let fake = Fake {
            files: (0..=MAX_FILES)
                .flat_map(|i| format!("{i}\0").into_bytes())
                .collect(),
            diff: vec![],
        };
        assert!(matches!(
            staged_ai_context(&fake, Path::new("/fixture")),
            Err(AiContextError::TooManyFiles)
        ));
    }

    #[test]
    fn omits_the_entire_patch_when_a_sensitive_path_is_staged() {
        let fake = Fake {
            files: b"src/a.rs\0.env.local\0".to_vec(),
            diff: b"secret=do-not-send".to_vec(),
        };
        let context = staged_ai_context(&fake, Path::new("/fixture")).unwrap();
        assert!(!context.text.contains("do-not-send"));
        assert!(context.text.contains("Sensitive staged paths omitted"));
    }

    struct RacingFake {
        index_reads: AtomicUsize,
    }

    impl RepositoryStatusGitExecutor for RacingFake {
        fn execute_repository_status(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout:
                    b"# branch.oid 0123456789012345678901234567890123456789\0# branch.head main\0"
                        .to_vec(),
                stderr: vec![],
            })
        }

        fn execute_index_entries(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            let read = self.index_reads.fetch_add(1, Ordering::Relaxed);
            Ok(GitOutput {
                stdout: format!("index-{read}").into_bytes(),
                stderr: vec![],
            })
        }
    }

    impl AiContextGitExecutor for RacingFake {
        fn staged_file_names(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: b"a.rs\0".to_vec(),
                stderr: vec![],
            })
        }

        fn staged_diff(&self, _: &Path) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: b"diff".to_vec(),
                stderr: vec![],
            })
        }
    }

    #[test]
    fn rejects_a_snapshot_when_the_index_changes_during_context_capture() {
        let fake = RacingFake {
            index_reads: AtomicUsize::new(0),
        };
        assert!(matches!(
            staged_ai_context(&fake, Path::new("/fixture")),
            Err(AiContextError::Stale)
        ));
    }
}
