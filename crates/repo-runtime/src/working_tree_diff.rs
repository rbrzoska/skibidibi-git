use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use app_domain::{StatusEntry, StatusEntryKind, WorkingTreeFileDiff};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, StatusParseError,
    parse_porcelain_v2_z,
};
use same_file::Handle;
use thiserror::Error;

use crate::RepositoryRuntime;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const PATCH_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const LITERAL_PATHSPEC_PREFIX: &str = ":(literal)";
const STATUS_ARGUMENTS: &[&str] = &[
    "status",
    "--porcelain=v2",
    "--branch",
    "-z",
    "--untracked-files=all",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingTreeDiffQuery {
    base: String,
    path: String,
    old_path: Option<String>,
    cached_only: bool,
}

impl WorkingTreeDiffQuery {
    fn new(base: &str, entry: &StatusEntry, cached_only: bool) -> Self {
        Self {
            base: base.to_owned(),
            path: entry.path.clone(),
            old_path: entry.original_path.clone(),
            cached_only,
        }
    }

    fn arguments(&self) -> Vec<OsString> {
        let mut arguments = vec!["diff".to_owned()];
        if self.cached_only {
            arguments.push("--cached".to_owned());
        }
        arguments.extend([
            "--no-color".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-textconv".to_owned(),
            "-M".to_owned(),
            "--unified=2147483647".to_owned(),
            self.base.clone(),
            "--".to_owned(),
        ]);
        let mut arguments = arguments
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

pub trait WorkingTreeDiffGitExecutor: Send + Sync {
    fn execute_working_tree_status(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
    fn execute_working_tree_diff(
        &self,
        repository: &Path,
        query: WorkingTreeDiffQuery,
    ) -> Result<GitOutput, GitRunError>;
    fn empty_tree_oid(&self, repository: &Path) -> Result<GitOutput, GitRunError>;
}

impl WorkingTreeDiffGitExecutor for GitRunner {
    fn execute_working_tree_status(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, STATUS_ARGUMENTS)
                .with_output_limits(PATCH_OUTPUT_LIMIT, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn execute_working_tree_diff(
        &self,
        repository: &Path,
        query: WorkingTreeDiffQuery,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(PATCH_OUTPUT_LIMIT, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn empty_tree_oid(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["hash-object", "-t", "tree", "--stdin"],
            )
            .with_stdin(Vec::new())
            .with_output_limits(128, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }
}

impl<E: WorkingTreeDiffGitExecutor> RepositoryRuntime<E> {
    pub fn working_tree_file_diff(
        &self,
        repository: &Path,
        path: &str,
        old_path: Option<&str>,
        kind: StatusEntryKind,
    ) -> Result<WorkingTreeFileDiff, WorkingTreeDiffRuntimeError> {
        working_tree_file_diff(&self.executor, repository, path, old_path, kind)
    }
}

#[derive(Debug, Error)]
pub enum WorkingTreeDiffRuntimeError {
    #[error("file path must be a non-empty relative repository path")]
    InvalidPath,
    #[error("the requested file is not present in the current working-tree status")]
    MissingStatusEntry,
    #[error("the requested paths do not match the current working-tree status entry")]
    StatusEntryMismatch,
    #[error("ignored or unmerged files cannot be rendered by this viewer")]
    UnsupportedStatus,
    #[error("untracked file is not a regular file inside the repository")]
    UnsafeUntrackedFile,
    #[error("working-tree file exceeded the configured {limit}-byte limit")]
    FileLimitExceeded { limit: usize },
    #[error("empty tree query returned an invalid object id")]
    InvalidEmptyTreeObjectId,
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    InvalidStatus(#[from] StatusParseError),
    #[error("failed to inspect the working-tree file: {0}")]
    Io(#[from] io::Error),
}

pub fn working_tree_file_diff<E: WorkingTreeDiffGitExecutor>(
    executor: &E,
    repository: &Path,
    path: &str,
    old_path: Option<&str>,
    kind: StatusEntryKind,
) -> Result<WorkingTreeFileDiff, WorkingTreeDiffRuntimeError> {
    validate_relative_path(path)?;
    if let Some(old_path) = old_path {
        validate_relative_path(old_path)?;
    }

    // Bind every dynamic path to a fresh porcelain record; never trust the UI status snapshot.
    let status_output = executor.execute_working_tree_status(repository)?;
    let status = parse_porcelain_v2_z(&status_output.stdout)?;
    let candidates = status
        .entries
        .iter()
        .filter(|entry| entry.path == path && entry.kind == kind)
        .collect::<Vec<_>>();
    let matches = candidates
        .iter()
        .copied()
        .filter(|entry| entry.original_path.as_deref() == old_path)
        .collect::<Vec<_>>();
    let [entry] = matches.as_slice() else {
        return Err(if candidates.is_empty() {
            WorkingTreeDiffRuntimeError::MissingStatusEntry
        } else {
            WorkingTreeDiffRuntimeError::StatusEntryMismatch
        });
    };
    match entry.kind {
        StatusEntryKind::Ignored | StatusEntryKind::Unmerged => {
            Err(WorkingTreeDiffRuntimeError::UnsupportedStatus)
        }
        StatusEntryKind::Untracked => untracked_file_diff(repository, entry),
        StatusEntryKind::Ordinary | StatusEntryKind::RenamedOrCopied => {
            let base = if status.branch.unborn {
                empty_tree_oid(executor, repository)?
            } else {
                "HEAD".to_owned()
            };
            let collides_with_untracked = entry.index_status == app_domain::StatusCode::Deleted
                && status.entries.iter().any(|candidate| {
                    candidate.path == entry.path && candidate.kind == StatusEntryKind::Untracked
                });
            let output = executor.execute_working_tree_diff(
                repository,
                WorkingTreeDiffQuery::new(&base, entry, collides_with_untracked),
            )?;
            let patch = String::from_utf8_lossy(&output.stdout).into_owned();
            Ok(WorkingTreeFileDiff {
                path: entry.path.clone(),
                old_path: entry.original_path.clone(),
                binary: is_binary_patch(&patch),
                patch,
                truncated: false,
            })
        }
    }
}

fn empty_tree_oid<E: WorkingTreeDiffGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Result<String, WorkingTreeDiffRuntimeError> {
    let output = executor.empty_tree_oid(repository)?;
    let oid = std::str::from_utf8(&output.stdout)
        .map_err(|_| WorkingTreeDiffRuntimeError::InvalidEmptyTreeObjectId)?
        .trim();
    if !valid_oid(oid) {
        return Err(WorkingTreeDiffRuntimeError::InvalidEmptyTreeObjectId);
    }
    Ok(oid.to_owned())
}

fn untracked_file_diff(
    repository: &Path,
    entry: &StatusEntry,
) -> Result<WorkingTreeFileDiff, WorkingTreeDiffRuntimeError> {
    let repository = fs::canonicalize(repository)?;
    let relative = validated_relative_path(&entry.path)?;
    let candidate = repository.join(relative);
    let link_metadata = fs::symlink_metadata(&candidate)
        .map_err(|_| WorkingTreeDiffRuntimeError::UnsafeUntrackedFile)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(WorkingTreeDiffRuntimeError::UnsafeUntrackedFile);
    }
    let canonical = fs::canonicalize(&candidate)
        .map_err(|_| WorkingTreeDiffRuntimeError::UnsafeUntrackedFile)?;
    if !canonical.starts_with(&repository) {
        return Err(WorkingTreeDiffRuntimeError::UnsafeUntrackedFile);
    }

    let checked_metadata = fs::metadata(&canonical)?;
    if !checked_metadata.is_file() {
        return Err(WorkingTreeDiffRuntimeError::UnsafeUntrackedFile);
    }
    let checked_handle = Handle::from_path(&canonical)?;
    let mut file = File::open(&canonical)?;
    let opened_metadata = file.metadata()?;
    let opened_handle = Handle::from_file(file.try_clone()?)?;
    if checked_handle != opened_handle {
        return Err(WorkingTreeDiffRuntimeError::UnsafeUntrackedFile);
    }
    if opened_metadata.len() > PATCH_OUTPUT_LIMIT as u64 {
        return Err(WorkingTreeDiffRuntimeError::FileLimitExceeded {
            limit: PATCH_OUTPUT_LIMIT,
        });
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(PATCH_OUTPUT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > PATCH_OUTPUT_LIMIT {
        return Err(WorkingTreeDiffRuntimeError::FileLimitExceeded {
            limit: PATCH_OUTPUT_LIMIT,
        });
    }

    let binary = bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
    let patch = if binary {
        binary_added_patch(&entry.path)
    } else {
        text_added_patch(
            &entry.path,
            std::str::from_utf8(&bytes).expect("checked UTF-8"),
        )
    };
    if patch.len() > PATCH_OUTPUT_LIMIT {
        return Err(WorkingTreeDiffRuntimeError::FileLimitExceeded {
            limit: PATCH_OUTPUT_LIMIT,
        });
    }
    Ok(WorkingTreeFileDiff {
        path: entry.path.clone(),
        old_path: None,
        patch,
        binary,
        truncated: false,
    })
}

fn validate_relative_path(path: &str) -> Result<(), WorkingTreeDiffRuntimeError> {
    validated_relative_path(path).map(|_| ())
}

fn validated_relative_path(path: &str) -> Result<PathBuf, WorkingTreeDiffRuntimeError> {
    if path.is_empty() || path.contains(['\0', '\u{fffd}']) {
        return Err(WorkingTreeDiffRuntimeError::InvalidPath);
    }
    let value = Path::new(path);
    if value.is_absolute()
        || !value
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(WorkingTreeDiffRuntimeError::InvalidPath);
    }
    Ok(value.to_path_buf())
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_binary_patch(patch: &str) -> bool {
    patch.lines().any(|line| {
        line == "GIT binary patch"
            || (line.starts_with("Binary files ") && line.ends_with(" differ"))
            || line.starts_with("Binary file ")
    })
}

fn binary_added_patch(path: &str) -> String {
    let display = quote_git_path(&format!("b/{path}"));
    format!(
        "diff --git /dev/null {display}\nnew file mode 100644\nBinary files /dev/null and {display} differ\n"
    )
}

fn text_added_patch(path: &str, contents: &str) -> String {
    let display = quote_git_path(&format!("b/{path}"));
    let mut patch = format!(
        "diff --git /dev/null {display}\nnew file mode 100644\n--- /dev/null\n+++ {display}\n"
    );
    if contents.is_empty() {
        return patch;
    }
    let line_count = contents.split_terminator('\n').count();
    patch.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
    for line in contents.split_inclusive('\n') {
        patch.push('+');
        patch.push_str(line);
        if !line.ends_with('\n') {
            patch.push('\n');
            patch.push_str("\\ No newline at end of file\n");
        }
    }
    patch
}

fn quote_git_path(path: &str) -> String {
    if path
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'))
    {
        return path.to_owned();
    }
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b'\\' => quoted.push_str("\\\\"),
            b'"' => quoted.push_str("\\\""),
            b'\n' => quoted.push_str("\\n"),
            b'\r' => quoted.push_str("\\r"),
            b'\t' => quoted.push_str("\\t"),
            0x20..=0x7e => quoted.push(char::from(byte)),
            _ => quoted.push_str(&format!("\\{:03o}", byte)),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_domain::{StatusCode, StatusEntryKind};
    use std::{fs, process::Command};

    fn git(repository: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(arguments)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository_with_commit() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("temporary repository");
        git(directory.path(), &["init", "-q", "-b", "main"]);
        git(directory.path(), &["config", "user.name", "Diff Test"]);
        git(
            directory.path(),
            &["config", "user.email", "diff@example.test"],
        );
        fs::write(directory.path().join("tracked.txt"), "base\n").unwrap();
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-qm", "base"]);
        directory
    }

    fn ordinary_entry(path: &str) -> StatusEntry {
        StatusEntry {
            kind: StatusEntryKind::Ordinary,
            path: path.to_owned(),
            original_path: None,
            index_status: StatusCode::Modified,
            worktree_status: StatusCode::Modified,
            submodule: None,
        }
    }

    #[test]
    fn query_uses_exact_flags_and_literal_pathspecs_for_injection_shaped_paths() {
        let mut entry = ordinary_entry("--output=/tmp/pwn :(glob)*");
        entry.kind = StatusEntryKind::RenamedOrCopied;
        entry.original_path = Some("old ;$() [name]".to_owned());
        let query = WorkingTreeDiffQuery::new("HEAD", &entry, false);
        let arguments = query
            .arguments()
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "-M",
                "--unified=2147483647",
                "HEAD",
                "--",
                ":(literal)old ;$() [name]",
                ":(literal)--output=/tmp/pwn :(glob)*",
            ]
        );
    }

    #[test]
    fn combines_staged_and_unstaged_changes_relative_to_head() {
        let repository = repository_with_commit();
        fs::write(repository.path().join("tracked.txt"), "staged\n").unwrap();
        git(repository.path(), &["add", "tracked.txt"]);
        fs::write(repository.path().join("tracked.txt"), "staged\nunstaged\n").unwrap();

        let diff = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "tracked.txt",
            None,
            StatusEntryKind::Ordinary,
        )
        .unwrap();

        assert!(diff.patch.contains("-base"));
        assert!(diff.patch.contains("+staged"));
        assert!(diff.patch.contains("+unstaged"));
        assert!(!diff.binary);
    }

    #[test]
    fn renders_rename_and_delete_using_fresh_status_paths() {
        let repository = repository_with_commit();
        git(repository.path(), &["mv", "tracked.txt", "renamed.txt"]);
        let renamed = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "renamed.txt",
            Some("tracked.txt"),
            StatusEntryKind::RenamedOrCopied,
        )
        .unwrap();
        assert!(renamed.patch.contains("rename from tracked.txt"));
        assert!(renamed.patch.contains("rename to renamed.txt"));

        git(repository.path(), &["reset", "--hard", "-q", "HEAD"]);
        fs::remove_file(repository.path().join("tracked.txt")).unwrap();
        let deleted = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "tracked.txt",
            None,
            StatusEntryKind::Ordinary,
        )
        .unwrap();
        assert!(deleted.patch.contains("deleted file mode"));
        assert!(deleted.patch.contains("-base"));
    }

    #[test]
    fn distinguishes_staged_delete_from_untracked_file_at_the_same_path() {
        let repository = repository_with_commit();
        git(repository.path(), &["rm", "--cached", "-q", "tracked.txt"]);

        let tracked_delete = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "tracked.txt",
            None,
            StatusEntryKind::Ordinary,
        )
        .unwrap();
        assert!(tracked_delete.patch.contains("deleted file mode"));
        assert!(tracked_delete.patch.contains("-base"));
        assert!(!tracked_delete.patch.contains("new file mode"));

        let untracked_add = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "tracked.txt",
            None,
            StatusEntryKind::Untracked,
        )
        .unwrap();
        assert!(untracked_add.patch.contains("new file mode"));
        assert!(untracked_add.patch.contains("+base"));
        assert!(!untracked_add.patch.contains("deleted file mode"));
    }

    #[test]
    fn renders_untracked_text_and_reports_untracked_binary_honestly() {
        let repository = repository_with_commit();
        fs::write(repository.path().join("new text.txt"), "first\nsecond").unwrap();
        let text = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "new text.txt",
            None,
            StatusEntryKind::Untracked,
        )
        .unwrap();
        assert!(!text.binary);
        assert!(text.patch.contains("+first\n+second\n"));
        assert!(text.patch.contains("\\ No newline at end of file"));
        assert!(text.patch.contains("\"b/new text.txt\""));

        fs::write(repository.path().join("image.bin"), [0_u8, 1, 2, 255]).unwrap();
        let binary = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "image.bin",
            None,
            StatusEntryKind::Untracked,
        )
        .unwrap();
        assert!(binary.binary);
        assert!(
            binary
                .patch
                .contains("Binary files /dev/null and b/image.bin differ")
        );
        assert!(!binary.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_untracked_symlink_and_lexical_traversal() {
        use std::os::unix::fs::symlink;

        let repository = repository_with_commit();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), repository.path().join("outside-link")).unwrap();

        assert!(matches!(
            working_tree_file_diff(
                &GitRunner::default(),
                repository.path(),
                "outside-link",
                None,
                StatusEntryKind::Untracked,
            ),
            Err(WorkingTreeDiffRuntimeError::UnsafeUntrackedFile)
        ));
        assert!(matches!(
            working_tree_file_diff(
                &GitRunner::default(),
                repository.path(),
                "../outside",
                None,
                StatusEntryKind::Untracked,
            ),
            Err(WorkingTreeDiffRuntimeError::InvalidPath)
        ));
    }

    #[test]
    fn open_file_identity_detects_replacement() {
        let first = tempfile::NamedTempFile::new().unwrap();
        let second = tempfile::NamedTempFile::new().unwrap();
        let checked = Handle::from_path(first.path()).unwrap();
        let same_opened = Handle::from_file(File::open(first.path()).unwrap()).unwrap();
        let replacement_opened = Handle::from_file(File::open(second.path()).unwrap()).unwrap();

        assert_eq!(checked, same_opened);
        assert_ne!(checked, replacement_opened);
    }

    #[test]
    fn unborn_repository_uses_the_repository_object_format_empty_tree() {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "-q", "-b", "main"]);
        fs::write(repository.path().join("first.txt"), "initial\n").unwrap();
        git(repository.path(), &["add", "first.txt"]);

        let diff = working_tree_file_diff(
            &GitRunner::default(),
            repository.path(),
            "first.txt",
            None,
            StatusEntryKind::Ordinary,
        )
        .unwrap();

        assert!(diff.patch.contains("new file mode"));
        assert!(diff.patch.contains("+initial"));
    }

    #[test]
    fn rejects_stale_or_tampered_old_path() {
        let repository = repository_with_commit();
        git(repository.path(), &["mv", "tracked.txt", "renamed.txt"]);

        assert!(matches!(
            working_tree_file_diff(
                &GitRunner::default(),
                repository.path(),
                "renamed.txt",
                Some("different.txt"),
                StatusEntryKind::RenamedOrCopied,
            ),
            Err(WorkingTreeDiffRuntimeError::StatusEntryMismatch)
        ));
        assert!(matches!(
            working_tree_file_diff(
                &GitRunner::default(),
                repository.path(),
                "missing.txt",
                None,
                StatusEntryKind::Ordinary,
            ),
            Err(WorkingTreeDiffRuntimeError::MissingStatusEntry)
        ));
    }
}
