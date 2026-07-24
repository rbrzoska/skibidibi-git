use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
    time::Duration,
};

use app_domain::{RepositoryStatus, StatusEntryKind};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{RepositoryRuntimeError, RepositoryStatusGitExecutor, repository_status};

const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FILES: usize = 200;
const MAX_CONTEXT_BYTES: usize = 1024 * 1024;
const MAX_UNTRACKED_FILE_BYTES: usize = 128 * 1024;
const MAX_GIT_OUTPUT: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskReviewCommit {
    pub oid: String,
    pub author: String,
    pub email: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskReviewContext {
    pub status: RepositoryStatus,
    pub branch: String,
    pub target_merged: bool,
    pub merge_base: String,
    pub changed_files: Vec<String>,
    pub my_commits: Vec<TaskReviewCommit>,
    pub text: String,
}

pub trait TaskReviewGitExecutor: RepositoryStatusGitExecutor {
    fn merge_base(
        &self,
        repository: &Path,
        head: &str,
        target: &str,
    ) -> Result<GitOutput, GitRunError>;
    fn changed_files(&self, repository: &Path, base: &str) -> Result<GitOutput, GitRunError>;
    fn task_diff(
        &self,
        repository: &Path,
        base: &str,
        paths: &[String],
    ) -> Result<GitOutput, GitRunError>;
    fn commits(&self, repository: &Path, base: &str, head: &str) -> Result<GitOutput, GitRunError>;
    fn user_config(&self, repository: &Path, key: &str) -> Result<Option<String>, GitRunError>;
}

impl TaskReviewGitExecutor for GitRunner {
    fn merge_base(
        &self,
        repository: &Path,
        head: &str,
        target: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            invocation(["merge-base", head, target], 128 * 1024),
        )
    }

    fn changed_files(&self, repository: &Path, base: &str) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            invocation(
                ["diff", "--name-only", "-z", "--no-renames", base, "--"],
                MAX_GIT_OUTPUT,
            ),
        )
    }

    fn task_diff(
        &self,
        repository: &Path,
        base: &str,
        paths: &[String],
    ) -> Result<GitOutput, GitRunError> {
        let mut arguments = vec![
            "diff".to_owned(),
            "--no-color".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-textconv".to_owned(),
            "-M".to_owned(),
            "--unified=3".to_owned(),
            base.to_owned(),
            "--".to_owned(),
        ];
        arguments.extend(paths.iter().map(|path| format!(":(literal){path}")));
        self.run(repository, invocation(arguments, MAX_GIT_OUTPUT))
    }

    fn commits(&self, repository: &Path, base: &str, head: &str) -> Result<GitOutput, GitRunError> {
        let range = format!("{base}..{head}");
        self.run(
            repository,
            invocation(
                [
                    "log",
                    "-z",
                    "--format=%H%x00%an%x00%ae%x00%s",
                    range.as_str(),
                ],
                4 * 1024 * 1024,
            ),
        )
    }

    fn user_config(&self, repository: &Path, key: &str) -> Result<Option<String>, GitRunError> {
        match self.run(repository, invocation(["config", "--get", key], 64 * 1024)) {
            Ok(output) => Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            )),
            Err(GitRunError::Unsuccessful { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn invocation<I, S>(arguments: I, stdout_limit: usize) -> GitInvocation
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    GitInvocation::new(GitInvocationPolicy::ReadOnly, arguments)
        .with_output_limits(stdout_limit, 256 * 1024)
        .with_timeout(QUERY_TIMEOUT)
}

#[derive(Debug, Error)]
pub enum TaskReviewContextError {
    #[error(transparent)]
    Repository(#[from] RepositoryRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error("task review requires an attached branch with a commit")]
    MissingHead,
    #[error("the requested target or HEAD object id is invalid")]
    InvalidObjectId,
    #[error("Git returned an invalid merge base")]
    InvalidMergeBase,
    #[error("Git returned an invalid changed-file list")]
    InvalidFileList,
    #[error("the task changes more than {MAX_FILES} files")]
    TooManyFiles,
    #[error("repository state changed while preparing the task review")]
    Stale,
}

pub fn task_review_context<E: TaskReviewGitExecutor>(
    executor: &E,
    repository: &Path,
    target_oid: &str,
) -> Result<TaskReviewContext, TaskReviewContextError> {
    if !valid_oid(target_oid) {
        return Err(TaskReviewContextError::InvalidObjectId);
    }
    let status = repository_status(executor, repository)?;
    let head = status
        .branch
        .oid
        .as_deref()
        .ok_or(TaskReviewContextError::MissingHead)?;
    let branch = status
        .branch
        .head
        .clone()
        .ok_or(TaskReviewContextError::MissingHead)?;
    if status.branch.detached || status.branch.unborn || !valid_oid(head) {
        return Err(TaskReviewContextError::MissingHead);
    }

    let merge_base = parse_oid(&executor.merge_base(repository, head, target_oid)?.stdout)?;
    let target_merged = merge_base.eq_ignore_ascii_case(target_oid);
    let mut files = parse_nul_paths(&executor.changed_files(repository, &merge_base)?.stdout)?;
    files.extend(
        status
            .entries
            .iter()
            .filter(|entry| entry.kind == StatusEntryKind::Untracked)
            .map(|entry| entry.path.clone()),
    );
    let files = files
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if files.len() > MAX_FILES {
        return Err(TaskReviewContextError::TooManyFiles);
    }

    let commits = parse_commits(&executor.commits(repository, &merge_base, head)?.stdout)?;
    let configured_email = executor
        .user_config(repository, "user.email")?
        .unwrap_or_default();
    let configured_name = executor
        .user_config(repository, "user.name")?
        .unwrap_or_default();
    let my_commits = commits
        .into_iter()
        .filter(|commit| {
            (!configured_email.is_empty() && commit.email.eq_ignore_ascii_case(&configured_email))
                || (configured_email.is_empty()
                    && !configured_name.is_empty()
                    && commit.author == configured_name)
        })
        .collect::<Vec<_>>();

    let sensitive = files
        .iter()
        .filter(|path| is_sensitive_path(path))
        .cloned()
        .collect::<BTreeSet<_>>();
    let untracked = status
        .entries
        .iter()
        .filter(|entry| entry.kind == StatusEntryKind::Untracked)
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let tracked_paths = files
        .iter()
        .filter(|path| !untracked.contains(path.as_str()) && !sensitive.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let diff = if tracked_paths.is_empty() {
        Vec::new()
    } else {
        executor
            .task_diff(repository, &merge_base, &tracked_paths)?
            .stdout
    };

    let mut text = format!(
        "Repository task review snapshot\nCurrent branch: {branch}\nHEAD: {head}\nTarget commit: {target_oid}\nMerge base: {merge_base}\nTarget fully merged: {target_merged}\nChanged files ({}):\n{}\n\nMy commits ({}):\n{}\n\nTask diff against merge base:\n",
        files.len(),
        files.join("\n"),
        my_commits.len(),
        my_commits
            .iter()
            .map(|commit| format!("{} {}", &commit.oid[..7], commit.summary))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    append_bounded(&mut text, &String::from_utf8_lossy(&diff));

    let mut untracked_budget = MAX_CONTEXT_BYTES.saturating_sub(text.len());
    for path in untracked.iter().filter(|path| !sensitive.contains(**path)) {
        if untracked_budget == 0 {
            break;
        }
        if let Some(content) = read_safe_untracked(repository, path, untracked_budget)? {
            let section = format!("\n\nUntracked file: {path}\n```\n{content}\n```");
            append_bounded(&mut text, &section);
            untracked_budget = MAX_CONTEXT_BYTES.saturating_sub(text.len());
        }
    }
    if !sensitive.is_empty() {
        append_bounded(
            &mut text,
            &format!(
                "\n\nSensitive paths omitted from review context:\n{}",
                sensitive.into_iter().collect::<Vec<_>>().join("\n")
            ),
        );
    }
    if text.len() >= MAX_CONTEXT_BYTES {
        let mut end = MAX_CONTEXT_BYTES.saturating_sub(40);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n\n[Review context truncated at 1 MiB]");
    }

    let after = repository_status(executor, repository)?;
    if status.branch.oid != after.branch.oid
        || status.index_fingerprint != after.index_fingerprint
        || status.worktree_fingerprint != after.worktree_fingerprint
    {
        return Err(TaskReviewContextError::Stale);
    }
    Ok(TaskReviewContext {
        status,
        branch,
        target_merged,
        merge_base,
        changed_files: files,
        my_commits,
        text,
    })
}

pub fn task_review_context_default(
    repository: &Path,
    target_oid: &str,
) -> Result<TaskReviewContext, TaskReviewContextError> {
    task_review_context(&GitRunner::default(), repository, target_oid)
}

fn parse_oid(output: &[u8]) -> Result<String, TaskReviewContextError> {
    let oid = String::from_utf8_lossy(output).trim().to_owned();
    valid_oid(&oid)
        .then_some(oid)
        .ok_or(TaskReviewContextError::InvalidMergeBase)
}

fn parse_nul_paths(output: &[u8]) -> Result<Vec<String>, TaskReviewContextError> {
    output
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| {
            std::str::from_utf8(value)
                .map(str::to_owned)
                .map_err(|_| TaskReviewContextError::InvalidFileList)
        })
        .collect()
}

fn parse_commits(output: &[u8]) -> Result<Vec<TaskReviewCommit>, TaskReviewContextError> {
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if fields.len() % 4 != 0 {
        return Err(TaskReviewContextError::InvalidFileList);
    }
    fields
        .chunks_exact(4)
        .map(|record| {
            let values = record
                .iter()
                .map(|value| std::str::from_utf8(value).map(str::to_owned))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| TaskReviewContextError::InvalidFileList)?;
            Ok(TaskReviewCommit {
                oid: values[0].clone(),
                author: values[1].clone(),
                email: values[2].clone(),
                summary: values[3].clone(),
            })
        })
        .collect()
}

fn append_bounded(target: &mut String, value: &str) {
    if target.len() >= MAX_CONTEXT_BYTES {
        return;
    }
    let remaining = MAX_CONTEXT_BYTES - target.len();
    if value.len() <= remaining {
        target.push_str(value);
        return;
    }
    let mut end = remaining;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    target.push_str(&value[..end]);
}

fn read_safe_untracked(
    repository: &Path,
    relative: &str,
    remaining: usize,
) -> Result<Option<String>, TaskReviewContextError> {
    let path = Path::new(relative);
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Ok(None);
    }
    let repository = fs::canonicalize(repository).map_err(GitRunError::Io)?;
    let joined = repository.join(path);
    let metadata = fs::symlink_metadata(&joined).map_err(GitRunError::Io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let parent = joined
        .parent()
        .and_then(|parent| fs::canonicalize(parent).ok());
    if !parent
        .as_ref()
        .is_some_and(|parent| parent.starts_with(&repository))
    {
        return Ok(None);
    }
    let limit = remaining.min(MAX_UNTRACKED_FILE_BYTES);
    let mut bytes = fs::read(&joined).map_err(GitRunError::Io)?;
    bytes.truncate(limit);
    if bytes.contains(&0) {
        return Ok(None);
    }
    while std::str::from_utf8(&bytes).is_err() {
        bytes.pop();
    }
    Ok(Some(
        String::from_utf8(bytes).expect("trimmed to UTF-8 boundary"),
    ))
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

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
