use std::{ffi::OsString, path::Path, time::Duration};

use app_domain::{RefComparison, RefComparisonFileDiff};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, HistoryParseError,
    parse_changed_files, parse_commit_list,
};
use thiserror::Error;

use crate::RepositoryRuntime;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const SMALL_OUTPUT_LIMIT: usize = 256 * 1024;
const LIST_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const PATCH_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const MAX_COMMITS: usize = 250;
const MAX_FILES: usize = 1_000;
const LITERAL_PATHSPEC_PREFIX: &str = ":(literal)";
const REF_FORMAT: &str = "--format=%(refname)%00%(objectname)%00%(symref)%00";
const COMMIT_FORMAT: &str = "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%s%x00%D";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefComparisonQuery {
    ResolveRef {
        full_name: String,
    },
    MergeBase {
        target_oid: String,
        source_oid: String,
    },
    AheadBehind {
        target_oid: String,
        source_oid: String,
    },
    Commits {
        target_oid: String,
        source_oid: String,
    },
    FileNames {
        merge_base_oid: String,
        source_oid: String,
    },
    FileNumbers {
        merge_base_oid: String,
        source_oid: String,
    },
    FilePatch {
        merge_base_oid: String,
        source_oid: String,
        path: String,
        old_path: Option<String>,
    },
}

impl RefComparisonQuery {
    fn arguments(&self) -> Vec<OsString> {
        let values = match self {
            Self::ResolveRef { full_name } => vec![
                "for-each-ref".to_owned(),
                REF_FORMAT.to_owned(),
                "--count=2".to_owned(),
                full_name.clone(),
            ],
            Self::MergeBase {
                target_oid,
                source_oid,
            } => vec![
                "merge-base".to_owned(),
                target_oid.clone(),
                source_oid.clone(),
            ],
            Self::AheadBehind {
                target_oid,
                source_oid,
            } => vec![
                "rev-list".to_owned(),
                "--left-right".to_owned(),
                "--count".to_owned(),
                format!("{target_oid}...{source_oid}"),
            ],
            Self::Commits {
                target_oid,
                source_oid,
            } => vec![
                "log".to_owned(),
                "-z".to_owned(),
                "--topo-order".to_owned(),
                format!("--max-count={}", MAX_COMMITS + 1),
                COMMIT_FORMAT.to_owned(),
                format!("{target_oid}..{source_oid}"),
            ],
            Self::FileNames {
                merge_base_oid,
                source_oid,
            } => vec![
                "diff".to_owned(),
                "--name-status".to_owned(),
                "-z".to_owned(),
                "-M".to_owned(),
                merge_base_oid.clone(),
                source_oid.clone(),
                "--".to_owned(),
            ],
            Self::FileNumbers {
                merge_base_oid,
                source_oid,
            } => vec![
                "diff".to_owned(),
                "--numstat".to_owned(),
                "-z".to_owned(),
                "-M".to_owned(),
                merge_base_oid.clone(),
                source_oid.clone(),
                "--".to_owned(),
            ],
            Self::FilePatch {
                merge_base_oid,
                source_oid,
                path,
                old_path,
            } => {
                let mut values = vec![
                    "diff".to_owned(),
                    "--no-color".to_owned(),
                    "--no-ext-diff".to_owned(),
                    "--no-textconv".to_owned(),
                    "-M".to_owned(),
                    "--unified=2147483647".to_owned(),
                    merge_base_oid.clone(),
                    source_oid.clone(),
                    "--".to_owned(),
                ];
                if let Some(old_path) = old_path {
                    values.push(format!("{LITERAL_PATHSPEC_PREFIX}{old_path}"));
                }
                values.push(format!("{LITERAL_PATHSPEC_PREFIX}{path}"));
                values
            }
        };
        values.into_iter().map(OsString::from).collect()
    }

    fn stdout_limit(&self) -> usize {
        match self {
            Self::ResolveRef { .. } | Self::MergeBase { .. } | Self::AheadBehind { .. } => {
                SMALL_OUTPUT_LIMIT
            }
            Self::Commits { .. } | Self::FileNames { .. } | Self::FileNumbers { .. } => {
                LIST_OUTPUT_LIMIT
            }
            Self::FilePatch { .. } => PATCH_OUTPUT_LIMIT,
        }
    }
}

pub trait RefComparisonGitExecutor: Send + Sync {
    fn execute_ref_comparison(
        &self,
        repository: &Path,
        query: RefComparisonQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl RefComparisonGitExecutor for GitRunner {
    fn execute_ref_comparison(
        &self,
        repository: &Path,
        query: RefComparisonQuery,
    ) -> Result<GitOutput, GitRunError> {
        let stdout_limit = query.stdout_limit();
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(stdout_limit, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum RefComparisonError {
    #[error("comparison refs must be exact local or remote branch names")]
    InvalidRef,
    #[error("comparison expected an exact 40- or 64-character object id")]
    InvalidObjectId,
    #[error("source branch changed while the comparison was loading; refresh and try again")]
    SourceMoved,
    #[error("target branch changed while the comparison was loading; refresh and try again")]
    TargetMoved,
    #[error("Git returned an invalid ref record")]
    InvalidRefOutput,
    #[error("Git returned invalid ahead/behind counts")]
    InvalidCounts,
    #[error("Git returned an invalid merge base")]
    InvalidMergeBase,
    #[error("file path must be non-empty and losslessly representable as UTF-8")]
    InvalidPath,
    #[error(transparent)]
    InvalidHistory(#[from] HistoryParseError),
    #[error(transparent)]
    Git(#[from] GitRunError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRef {
    full_name: String,
    oid: String,
}

impl<E: RefComparisonGitExecutor> RepositoryRuntime<E> {
    #[allow(clippy::too_many_arguments)]
    pub fn compare_refs(
        &self,
        repository: &Path,
        source_full_name: &str,
        expected_source_oid: &str,
        target_full_name: &str,
        expected_target_oid: &str,
    ) -> Result<RefComparison, RefComparisonError> {
        compare_refs(
            &self.executor,
            repository,
            source_full_name,
            expected_source_oid,
            target_full_name,
            expected_target_oid,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn compare_ref_file_diff(
        &self,
        repository: &Path,
        source_full_name: &str,
        expected_source_oid: &str,
        target_full_name: &str,
        expected_target_oid: &str,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<RefComparisonFileDiff, RefComparisonError> {
        compare_ref_file_diff(
            &self.executor,
            repository,
            source_full_name,
            expected_source_oid,
            target_full_name,
            expected_target_oid,
            path,
            old_path,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compare_refs<E: RefComparisonGitExecutor>(
    executor: &E,
    repository: &Path,
    source_full_name: &str,
    expected_source_oid: &str,
    target_full_name: &str,
    expected_target_oid: &str,
) -> Result<RefComparison, RefComparisonError> {
    validate_inputs(
        source_full_name,
        expected_source_oid,
        target_full_name,
        expected_target_oid,
    )?;
    let (source, target) = verify_snapshot(
        executor,
        repository,
        source_full_name,
        expected_source_oid,
        target_full_name,
        expected_target_oid,
    )?;
    let merge_base_oid = resolve_merge_base(executor, repository, &target.oid, &source.oid)?;
    let (ahead, behind) = ahead_behind(executor, repository, &target.oid, &source.oid)?;

    let commit_output = executor.execute_ref_comparison(
        repository,
        RefComparisonQuery::Commits {
            target_oid: target.oid.clone(),
            source_oid: source.oid.clone(),
        },
    )?;
    let mut commits = parse_commit_list(&commit_output.stdout)?;
    let commits_truncated = commits.len() > MAX_COMMITS;
    commits.truncate(MAX_COMMITS);

    let names = executor.execute_ref_comparison(
        repository,
        RefComparisonQuery::FileNames {
            merge_base_oid: merge_base_oid.clone(),
            source_oid: source.oid.clone(),
        },
    )?;
    let numbers = executor.execute_ref_comparison(
        repository,
        RefComparisonQuery::FileNumbers {
            merge_base_oid: merge_base_oid.clone(),
            source_oid: source.oid.clone(),
        },
    )?;
    let mut files = parse_changed_files(&names.stdout, &numbers.stdout)?;
    let files_truncated = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);

    verify_snapshot(
        executor,
        repository,
        source_full_name,
        expected_source_oid,
        target_full_name,
        expected_target_oid,
    )?;

    Ok(RefComparison {
        source_full_name: source.full_name,
        source_oid: source.oid,
        target_full_name: target.full_name,
        target_oid: target.oid,
        merge_base_oid,
        ahead,
        behind,
        commits,
        commits_truncated,
        files,
        files_truncated,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn compare_ref_file_diff<E: RefComparisonGitExecutor>(
    executor: &E,
    repository: &Path,
    source_full_name: &str,
    expected_source_oid: &str,
    target_full_name: &str,
    expected_target_oid: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<RefComparisonFileDiff, RefComparisonError> {
    validate_inputs(
        source_full_name,
        expected_source_oid,
        target_full_name,
        expected_target_oid,
    )?;
    if !valid_path(path) || old_path.is_some_and(|value| !valid_path(value)) {
        return Err(RefComparisonError::InvalidPath);
    }
    let (source, target) = verify_snapshot(
        executor,
        repository,
        source_full_name,
        expected_source_oid,
        target_full_name,
        expected_target_oid,
    )?;
    let merge_base_oid = resolve_merge_base(executor, repository, &target.oid, &source.oid)?;
    let output = executor.execute_ref_comparison(
        repository,
        RefComparisonQuery::FilePatch {
            merge_base_oid,
            source_oid: source.oid.clone(),
            path: path.to_owned(),
            old_path: old_path.filter(|value| *value != path).map(str::to_owned),
        },
    )?;
    verify_snapshot(
        executor,
        repository,
        source_full_name,
        expected_source_oid,
        target_full_name,
        expected_target_oid,
    )?;

    let patch = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(RefComparisonFileDiff {
        source_full_name: source.full_name,
        source_oid: source.oid,
        target_full_name: target.full_name,
        target_oid: target.oid,
        path: path.to_owned(),
        old_path: old_path.map(str::to_owned),
        binary: is_binary_patch(&patch),
        patch,
        truncated: false,
    })
}

fn validate_inputs(
    source_full_name: &str,
    expected_source_oid: &str,
    target_full_name: &str,
    expected_target_oid: &str,
) -> Result<(), RefComparisonError> {
    if !valid_branch_ref(source_full_name) || !valid_branch_ref(target_full_name) {
        return Err(RefComparisonError::InvalidRef);
    }
    if !valid_oid(expected_source_oid) || !valid_oid(expected_target_oid) {
        return Err(RefComparisonError::InvalidObjectId);
    }
    Ok(())
}

fn verify_snapshot<E: RefComparisonGitExecutor>(
    executor: &E,
    repository: &Path,
    source_full_name: &str,
    expected_source_oid: &str,
    target_full_name: &str,
    expected_target_oid: &str,
) -> Result<(ResolvedRef, ResolvedRef), RefComparisonError> {
    let source =
        resolve_exact_ref(executor, repository, source_full_name).map_err(map_source_ref_error)?;
    if !source.oid.eq_ignore_ascii_case(expected_source_oid) {
        return Err(RefComparisonError::SourceMoved);
    }
    let target =
        resolve_exact_ref(executor, repository, target_full_name).map_err(map_target_ref_error)?;
    if !target.oid.eq_ignore_ascii_case(expected_target_oid) {
        return Err(RefComparisonError::TargetMoved);
    }
    Ok((source, target))
}

fn map_source_ref_error(error: RefComparisonError) -> RefComparisonError {
    match error {
        RefComparisonError::InvalidRefOutput => RefComparisonError::SourceMoved,
        error => error,
    }
}

fn map_target_ref_error(error: RefComparisonError) -> RefComparisonError {
    match error {
        RefComparisonError::InvalidRefOutput => RefComparisonError::TargetMoved,
        error => error,
    }
}

fn resolve_exact_ref<E: RefComparisonGitExecutor>(
    executor: &E,
    repository: &Path,
    full_name: &str,
) -> Result<ResolvedRef, RefComparisonError> {
    let output = executor.execute_ref_comparison(
        repository,
        RefComparisonQuery::ResolveRef {
            full_name: full_name.to_owned(),
        },
    )?;
    let mut match_found = None;
    for line in output.stdout.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split(|byte| *byte == 0);
        let name = fields.next().ok_or(RefComparisonError::InvalidRefOutput)?;
        let oid = fields.next().ok_or(RefComparisonError::InvalidRefOutput)?;
        let symbolic_target = fields.next().ok_or(RefComparisonError::InvalidRefOutput)?;
        if fields.next() != Some(&[][..]) || fields.next().is_some() {
            return Err(RefComparisonError::InvalidRefOutput);
        }
        let name = std::str::from_utf8(name).map_err(|_| RefComparisonError::InvalidRefOutput)?;
        if name != full_name {
            continue;
        }
        let oid = std::str::from_utf8(oid).map_err(|_| RefComparisonError::InvalidRefOutput)?;
        if !valid_oid(oid) || !symbolic_target.is_empty() || match_found.is_some() {
            return Err(RefComparisonError::InvalidRefOutput);
        }
        match_found = Some(ResolvedRef {
            full_name: name.to_owned(),
            oid: oid.to_owned(),
        });
    }
    match_found.ok_or(RefComparisonError::InvalidRefOutput)
}

fn resolve_merge_base<E: RefComparisonGitExecutor>(
    executor: &E,
    repository: &Path,
    target_oid: &str,
    source_oid: &str,
) -> Result<String, RefComparisonError> {
    let output = executor.execute_ref_comparison(
        repository,
        RefComparisonQuery::MergeBase {
            target_oid: target_oid.to_owned(),
            source_oid: source_oid.to_owned(),
        },
    )?;
    let oid = std::str::from_utf8(&output.stdout)
        .map_err(|_| RefComparisonError::InvalidMergeBase)?
        .trim();
    if !valid_oid(oid) {
        return Err(RefComparisonError::InvalidMergeBase);
    }
    Ok(oid.to_owned())
}

fn ahead_behind<E: RefComparisonGitExecutor>(
    executor: &E,
    repository: &Path,
    target_oid: &str,
    source_oid: &str,
) -> Result<(u64, u64), RefComparisonError> {
    let output = executor.execute_ref_comparison(
        repository,
        RefComparisonQuery::AheadBehind {
            target_oid: target_oid.to_owned(),
            source_oid: source_oid.to_owned(),
        },
    )?;
    let text =
        std::str::from_utf8(&output.stdout).map_err(|_| RefComparisonError::InvalidCounts)?;
    let mut values = text.split_whitespace();
    let behind = values
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(RefComparisonError::InvalidCounts)?;
    let ahead = values
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(RefComparisonError::InvalidCounts)?;
    if values.next().is_some() {
        return Err(RefComparisonError::InvalidCounts);
    }
    Ok((ahead, behind))
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_branch_ref(value: &str) -> bool {
    let Some(name) = value
        .strip_prefix("refs/heads/")
        .or_else(|| value.strip_prefix("refs/remotes/"))
    else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('/')
        && !name.ends_with(['/', '.'])
        && !["//", "..", "@{"].iter().any(|part| name.contains(part))
        && name.split('/').all(|component| {
            !component.is_empty()
                && !component.starts_with('.')
                && !component.ends_with(".lock")
                && !component
                    .bytes()
                    .any(|byte| byte <= b' ' || byte == 0x7f || b"~^:?*[\\".contains(&byte))
        })
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

    #[test]
    fn accepts_only_exact_local_and_remote_branch_refs() {
        for valid in [
            "refs/heads/main",
            "refs/heads/feature/task",
            "refs/remotes/origin/main",
        ] {
            assert!(valid_branch_ref(valid), "{valid}");
        }
        for invalid in [
            "main",
            "HEAD",
            "refs/tags/v1",
            "refs/heads/../main",
            "refs/heads/a.lock",
            "refs/remotes/origin/a*[b]",
            "refs/remotes/origin/a\nbranch",
        ] {
            assert!(!valid_branch_ref(invalid), "{invalid}");
        }
    }

    #[test]
    fn patch_query_uses_literal_pathspecs_after_the_separator() {
        let oid_a = "a".repeat(40);
        let oid_b = "b".repeat(40);
        let query = RefComparisonQuery::FilePatch {
            merge_base_oid: oid_a.clone(),
            source_oid: oid_b.clone(),
            path: "--output=/tmp/pwn :(glob)*".to_owned(),
            old_path: None,
        };
        let arguments = query
            .arguments()
            .into_iter()
            .map(|value| value.into_string().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            &arguments[arguments.len() - 4..],
            [
                oid_a,
                oid_b,
                "--".to_owned(),
                ":(literal)--output=/tmp/pwn :(glob)*".to_owned()
            ]
        );
    }
}
