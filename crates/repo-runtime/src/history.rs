use std::{ffi::OsString, path::Path, time::Duration};

use app_domain::{CommitDetails, CommitHistoryPage};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, HistoryParseError,
    parse_changed_files, parse_commit_details_header, parse_commit_list,
};
use thiserror::Error;

use crate::RepositoryRuntime;

pub const DEFAULT_HISTORY_PAGE_SIZE: usize = 100;
pub const MAX_HISTORY_PAGE_SIZE: usize = 250;
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const HEAD_OUTPUT_LIMIT: usize = 128;
const DETAIL_HEADER_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const HISTORY_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryQuery {
    ResolveHead,
    Page {
        snapshot_head: String,
        offset: usize,
        max_count: usize,
    },
    DetailsHeader {
        oid: String,
    },
    DetailsNames {
        oid: String,
    },
    DetailsNumbers {
        oid: String,
    },
}

impl HistoryQuery {
    fn arguments(&self) -> Vec<OsString> {
        let values = match self {
            Self::ResolveHead => vec![
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--quiet".to_owned(),
                "HEAD".to_owned(),
            ],
            Self::Page {
                snapshot_head,
                offset,
                max_count,
            } => vec![
                "log".to_owned(),
                "-z".to_owned(),
                "--topo-order".to_owned(),
                format!("--max-count={max_count}"),
                format!("--skip={offset}"),
                "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%s%x00%D".to_owned(),
                snapshot_head.clone(),
            ],
            Self::DetailsHeader { oid } => vec![
                "show".to_owned(),
                "-s".to_owned(),
                "-z".to_owned(),
                "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%s%x00%D%x00%B".to_owned(),
                oid.clone(),
                "--".to_owned(),
            ],
            Self::DetailsNames { oid } => vec![
                "show".to_owned(),
                "--format=".to_owned(),
                "--first-parent".to_owned(),
                "--name-status".to_owned(),
                "-z".to_owned(),
                "-M".to_owned(),
                oid.clone(),
                "--".to_owned(),
            ],
            Self::DetailsNumbers { oid } => vec![
                "show".to_owned(),
                "--format=".to_owned(),
                "--first-parent".to_owned(),
                "--numstat".to_owned(),
                "-z".to_owned(),
                "-M".to_owned(),
                oid.clone(),
                "--".to_owned(),
            ],
        };
        values.into_iter().map(OsString::from).collect()
    }

    fn stdout_limit(&self) -> usize {
        match self {
            Self::ResolveHead => HEAD_OUTPUT_LIMIT,
            Self::DetailsHeader { .. } => DETAIL_HEADER_OUTPUT_LIMIT,
            Self::Page { .. } | Self::DetailsNames { .. } | Self::DetailsNumbers { .. } => {
                HISTORY_OUTPUT_LIMIT
            }
        }
    }
}

pub trait HistoryGitExecutor: Send + Sync {
    fn execute_history(
        &self,
        repository: &Path,
        query: HistoryQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl HistoryGitExecutor for GitRunner {
    fn execute_history(
        &self,
        repository: &Path,
        query: HistoryQuery,
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

impl<E: HistoryGitExecutor> RepositoryRuntime<E> {
    pub fn history_page(
        &self,
        repository: &Path,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<CommitHistoryPage, HistoryRuntimeError> {
        history_page(&self.executor, repository, page_size, cursor)
    }

    pub fn commit_details(
        &self,
        repository: &Path,
        oid: &str,
    ) -> Result<CommitDetails, HistoryRuntimeError> {
        commit_details(&self.executor, repository, oid)
    }
}

#[derive(Debug, Error)]
pub enum HistoryRuntimeError {
    #[error("history page size must be between 1 and {MAX_HISTORY_PAGE_SIZE}")]
    InvalidPageSize,
    #[error("invalid history cursor")]
    InvalidCursor,
    #[error("invalid commit object id")]
    InvalidObjectId,
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    Parse(#[from] HistoryParseError),
}

pub fn history_page<E: HistoryGitExecutor>(
    executor: &E,
    repository: &Path,
    page_size: usize,
    cursor: Option<&str>,
) -> Result<CommitHistoryPage, HistoryRuntimeError> {
    if !(1..=MAX_HISTORY_PAGE_SIZE).contains(&page_size) {
        return Err(HistoryRuntimeError::InvalidPageSize);
    }
    let (snapshot_head, offset) = match cursor {
        Some(cursor) => parse_cursor(cursor)?,
        None => match resolve_head(executor, repository)? {
            Some(head) => (head, 0),
            None => {
                return Ok(CommitHistoryPage {
                    commits: Vec::new(),
                    next_cursor: None,
                });
            }
        },
    };
    let output = executor.execute_history(
        repository,
        HistoryQuery::Page {
            snapshot_head: snapshot_head.clone(),
            offset,
            max_count: page_size + 1,
        },
    )?;
    let mut commits = parse_commit_list(&output.stdout)?;
    let has_more = commits.len() > page_size;
    commits.truncate(page_size);
    let next_cursor = if has_more {
        let next_offset = offset
            .checked_add(commits.len())
            .ok_or(HistoryRuntimeError::InvalidCursor)?;
        Some(format!("{snapshot_head}:{next_offset}"))
    } else {
        None
    };
    Ok(CommitHistoryPage {
        commits,
        next_cursor,
    })
}

fn resolve_head<E: HistoryGitExecutor>(
    executor: &E,
    repository: &Path,
) -> Result<Option<String>, HistoryRuntimeError> {
    match executor.execute_history(repository, HistoryQuery::ResolveHead) {
        Ok(output) => {
            let oid = std::str::from_utf8(&output.stdout)
                .map_err(|_| HistoryRuntimeError::InvalidObjectId)?
                .trim();
            if !valid_oid(oid) {
                return Err(HistoryRuntimeError::InvalidObjectId);
            }
            Ok(Some(oid.to_owned()))
        }
        Err(GitRunError::Unsuccessful {
            code: Some(1),
            ref stderr,
        }) if stderr.is_empty() => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn parse_cursor(cursor: &str) -> Result<(String, usize), HistoryRuntimeError> {
    let (snapshot_head, offset) = cursor
        .split_once(':')
        .ok_or(HistoryRuntimeError::InvalidCursor)?;
    if !valid_oid(snapshot_head)
        || offset.is_empty()
        || !offset.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(HistoryRuntimeError::InvalidCursor);
    }
    let offset = offset
        .parse()
        .map_err(|_| HistoryRuntimeError::InvalidCursor)?;
    Ok((snapshot_head.to_owned(), offset))
}

pub fn commit_details<E: HistoryGitExecutor>(
    executor: &E,
    repository: &Path,
    oid: &str,
) -> Result<CommitDetails, HistoryRuntimeError> {
    if !valid_oid(oid) {
        return Err(HistoryRuntimeError::InvalidObjectId);
    }
    let mut details = parse_commit_details_header(
        &executor
            .execute_history(
                repository,
                HistoryQuery::DetailsHeader {
                    oid: oid.to_owned(),
                },
            )?
            .stdout,
    )?;
    let names = executor.execute_history(
        repository,
        HistoryQuery::DetailsNames {
            oid: oid.to_owned(),
        },
    )?;
    let numbers = executor.execute_history(
        repository,
        HistoryQuery::DetailsNumbers {
            oid: oid.to_owned(),
        },
    )?;
    details.files = parse_changed_files(&names.stdout, &numbers.stdout)?;
    Ok(details)
}

fn valid_oid(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use git_core::GitOutput;
    use std::{fs, process::Command, sync::Mutex};
    const OID: &str = "0123456789012345678901234567890123456789";

    struct Executor {
        queries: Mutex<Vec<HistoryQuery>>,
        outputs: Mutex<Vec<Vec<u8>>>,
    }
    impl HistoryGitExecutor for Executor {
        fn execute_history(
            &self,
            _repository: &Path,
            query: HistoryQuery,
        ) -> Result<GitOutput, GitRunError> {
            self.queries.lock().unwrap().push(query);
            Ok(GitOutput {
                stdout: self.outputs.lock().unwrap().remove(0),
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn cursor_keeps_snapshot_and_advances_offset() {
        let record = format!("{OID}\0\0A\0a@b\02026-01-01T00:00:00Z\0s\0\0").into_bytes();
        let output = [record.as_slice(), record.as_slice(), record.as_slice()].concat();
        let executor = Executor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(vec![output]),
        };
        let page =
            history_page(&executor, Path::new("/repo"), 2, Some(&format!("{OID}:42"))).unwrap();
        assert_eq!(
            page.next_cursor.as_deref(),
            Some(format!("{OID}:44").as_str())
        );
        assert_eq!(
            executor.queries.lock().unwrap().as_slice(),
            &[HistoryQuery::Page {
                snapshot_head: OID.to_owned(),
                offset: 42,
                max_count: 3
            }]
        );
    }

    #[test]
    fn page_query_uses_one_deterministic_order_and_bounded_argv() {
        let query = HistoryQuery::Page {
            snapshot_head: OID.to_owned(),
            offset: 7,
            max_count: 11,
        };
        let arguments = query.arguments();
        let arguments = arguments
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>();

        assert!(arguments.contains(&"--topo-order"));
        assert!(!arguments.contains(&"--date-order"));
        assert!(arguments.contains(&"--max-count=11"));
        assert!(arguments.contains(&"--skip=7"));
        assert_eq!(arguments.last(), Some(&OID));
        assert_eq!(query.stdout_limit(), HISTORY_OUTPUT_LIMIT);
    }

    #[test]
    fn malformed_snapshot_cursor_is_rejected_before_git() {
        let executor = Executor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(Vec::new()),
        };
        for cursor in [OID, "abc:1", "0123456789012345678901234567890123456789:-1"] {
            assert!(matches!(
                history_page(&executor, Path::new("/repo"), 2, Some(cursor)),
                Err(HistoryRuntimeError::InvalidCursor)
            ));
        }
        assert!(executor.queries.lock().unwrap().is_empty());
    }

    #[test]
    fn rejects_unbounded_pages_without_running_git() {
        let executor = Executor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            history_page(&executor, Path::new("/repo"), 251, None),
            Err(HistoryRuntimeError::InvalidPageSize)
        ));
        assert!(executor.queries.lock().unwrap().is_empty());
    }

    #[test]
    fn details_use_read_only_show_commands_and_combine_file_stats() {
        let header = format!(
            "{OID}\0\0Author\0a@b\02026-01-01T00:00:00Z\0subject\0HEAD -> main\0subject\nbody\n\0"
        )
        .into_bytes();
        let executor = Executor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(vec![
                header,
                b"R100\0old\0new\0".to_vec(),
                b"2\t1\t\0old\0new\0".to_vec(),
            ]),
        };

        let details = commit_details(&executor, Path::new("/repo"), OID).unwrap();

        assert_eq!(details.full_message, "subject\nbody\n");
        assert_eq!(details.files[0].old_path.as_deref(), Some("old"));
        assert_eq!(
            executor.queries.lock().unwrap().as_slice(),
            &[
                HistoryQuery::DetailsHeader {
                    oid: OID.to_owned()
                },
                HistoryQuery::DetailsNames {
                    oid: OID.to_owned()
                },
                HistoryQuery::DetailsNumbers {
                    oid: OID.to_owned()
                },
            ]
        );
    }

    #[test]
    fn detail_file_queries_use_the_same_first_parent_merge_semantics() {
        for query in [
            HistoryQuery::DetailsNames {
                oid: OID.to_owned(),
            },
            HistoryQuery::DetailsNumbers {
                oid: OID.to_owned(),
            },
        ] {
            let arguments = query.arguments();
            let arguments = arguments
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>();
            assert!(arguments.contains(&"--first-parent"));
        }
    }

    #[test]
    fn snapshot_offset_pages_match_full_traversal_on_merge_graph() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        git(directory.path(), &["config", "user.name", "History Test"]);
        git(
            directory.path(),
            &["config", "user.email", "history@example.test"],
        );
        fs::write(directory.path().join("base"), "base").unwrap();
        git(directory.path(), &["add", "base"]);
        git(directory.path(), &["commit", "-m", "base"]);
        git(directory.path(), &["branch", "side"]);
        fs::write(directory.path().join("main"), "main").unwrap();
        git(directory.path(), &["add", "main"]);
        git(directory.path(), &["commit", "-m", "main"]);
        git(directory.path(), &["checkout", "side"]);
        fs::write(directory.path().join("side"), "side").unwrap();
        git(directory.path(), &["add", "side"]);
        git(directory.path(), &["commit", "-m", "side"]);
        git(directory.path(), &["checkout", "main"]);
        git(
            directory.path(),
            &["merge", "--no-ff", "side", "-m", "merge"],
        );

        let runtime = RepositoryRuntime::default();
        let merge_oid = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(directory.path())
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let merge_details = runtime
            .commit_details(directory.path(), merge_oid.trim())
            .expect("merge commit details use one deterministic parent");
        assert_eq!(merge_details.files.len(), 1);
        assert_eq!(merge_details.files[0].path, "side");
        let expected = runtime.history_page(directory.path(), 250, None).unwrap();
        let mut actual = Vec::new();
        let mut cursor = None;
        loop {
            let page = runtime
                .history_page(directory.path(), 2, cursor.as_deref())
                .unwrap();
            actual.extend(page.commits.into_iter().map(|commit| commit.oid));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            actual,
            expected
                .commits
                .into_iter()
                .map(|commit| commit.oid)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unborn_repository_has_empty_history() {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);

        let page = RepositoryRuntime::default()
            .history_page(directory.path(), 25, None)
            .unwrap();

        assert!(page.commits.is_empty());
        assert!(page.next_cursor.is_none());
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
}
