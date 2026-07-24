use std::{collections::HashSet, ffi::OsString, path::Path, time::Duration};

use app_domain::{CommitHistoryPage, CommitRelation, RepositoryBranchKind};
use git_core::{GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner};
use thiserror::Error;

use crate::{
    HistoryGitExecutor, HistoryRuntimeError, NavigationGitExecutor, NavigationRuntimeError,
    RepositoryRuntime, history_page,
};

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const REV_LIST_OUTPUT_LIMIT: usize = 128 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
// Keep this in sync with the paged rev-list policy in git-core. The bound prevents a forged
// opaque cursor from forcing Git to traverse an effectively unbounded number of commits.
const MAX_PREVIEW_OFFSET: usize = 10_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchPreviewQuery {
    UniqueCommits {
        branch_oid: String,
        target_oid: String,
        first_parent: bool,
        skip: usize,
        max_count: usize,
    },
}

impl BranchPreviewQuery {
    fn arguments(&self) -> Vec<OsString> {
        match self {
            Self::UniqueCommits {
                branch_oid,
                target_oid,
                first_parent,
                skip,
                max_count,
            } => {
                let mut arguments = vec![
                    OsString::from("rev-list"),
                    OsString::from("--topo-order"),
                    OsString::from(format!("--skip={skip}")),
                    OsString::from(format!("--max-count={max_count}")),
                ];
                if *first_parent {
                    arguments.push(OsString::from("--first-parent"));
                }
                arguments.push(OsString::from(branch_oid));
                arguments.push(OsString::from(format!("^{target_oid}")));
                arguments
            }
        }
    }
}

pub trait BranchPreviewGitExecutor: Send + Sync {
    fn execute_branch_preview(
        &self,
        repository: &Path,
        query: BranchPreviewQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl BranchPreviewGitExecutor for GitRunner {
    fn execute_branch_preview(
        &self,
        repository: &Path,
        query: BranchPreviewQuery,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(REV_LIST_OUTPUT_LIMIT, STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum BranchPreviewError {
    #[error("branch preview is only available for an exact local branch")]
    InvalidBranch,
    #[error("branch changed while the preview was being opened; refresh and try again")]
    BranchMoved,
    #[error("target branch changed while the preview was being opened; refresh and try again")]
    TargetMoved,
    #[error("history cursor does not belong to the previewed branch")]
    InvalidCursor,
    #[error("Git returned an invalid object id while classifying commits")]
    InvalidObjectId,
    #[error(transparent)]
    Navigation(#[from] NavigationRuntimeError),
    #[error(transparent)]
    History(#[from] HistoryRuntimeError),
    #[error(transparent)]
    Git(#[from] GitRunError),
}

impl<E> RepositoryRuntime<E>
where
    E: BranchPreviewGitExecutor + HistoryGitExecutor + NavigationGitExecutor,
{
    #[allow(clippy::too_many_arguments)]
    pub fn branch_history_page(
        &self,
        repository: &Path,
        branch_full_name: &str,
        expected_branch_oid: &str,
        target_full_name: &str,
        expected_target_oid: &str,
        page_size: usize,
        cursor: Option<&str>,
    ) -> Result<CommitHistoryPage, BranchPreviewError> {
        validate_local_ref(branch_full_name)?;
        validate_local_ref(target_full_name)?;
        validate_oid(expected_branch_oid)?;
        validate_oid(expected_target_oid)?;

        self.verify_snapshot(
            repository,
            branch_full_name,
            expected_branch_oid,
            target_full_name,
            expected_target_oid,
        )?;

        let preview_cursor = match cursor {
            Some(cursor) => parse_preview_cursor(cursor, expected_branch_oid)?,
            None => PreviewCursor::new(expected_branch_oid),
        };
        let history_cursor = format!(
            "{}:{}",
            preview_cursor.branch_oid, preview_cursor.history_offset
        );
        let mut page = history_page(&self.executor, repository, page_size, Some(&history_cursor))?;

        let max_count = page.commits.len();
        let unique = self.commit_set(
            repository,
            expected_branch_oid,
            expected_target_oid,
            false,
            preview_cursor.unique_offset,
            max_count,
        )?;
        let first_parent = self.commit_set(
            repository,
            expected_branch_oid,
            expected_target_oid,
            true,
            preview_cursor.first_parent_offset,
            max_count,
        )?;
        let mut page_unique_count = 0_usize;
        let mut page_first_parent_count = 0_usize;
        for commit in &mut page.commits {
            let is_unique = unique.contains(&commit.oid);
            let is_first_parent = first_parent.contains(&commit.oid);
            page_unique_count += usize::from(is_unique);
            page_first_parent_count += usize::from(is_first_parent);
            commit.relation = Some(if !is_unique {
                CommitRelation::Base
            } else if commit.parents.len() > 1 || !is_first_parent {
                CommitRelation::Merge
            } else {
                CommitRelation::Task
            });
        }

        page.next_cursor = match page.next_cursor {
            Some(_) => Some(
                PreviewCursor {
                    branch_oid: preview_cursor.branch_oid.clone(),
                    history_offset: advance_cursor_offset(
                        preview_cursor.history_offset,
                        page.commits.len(),
                    )?,
                    unique_offset: advance_cursor_offset(
                        preview_cursor.unique_offset,
                        page_unique_count,
                    )?,
                    first_parent_offset: advance_cursor_offset(
                        preview_cursor.first_parent_offset,
                        page_first_parent_count,
                    )?,
                }
                .encode(),
            ),
            None => None,
        };

        // Resolve the refs again after all history queries. The Git operations use immutable OIDs,
        // so their output is internally consistent, but the UI must not present it as a preview of
        // a branch name that moved while the page was loading.
        self.verify_snapshot(
            repository,
            branch_full_name,
            expected_branch_oid,
            target_full_name,
            expected_target_oid,
        )?;
        Ok(page)
    }

    fn verify_snapshot(
        &self,
        repository: &Path,
        branch_full_name: &str,
        expected_branch_oid: &str,
        target_full_name: &str,
        expected_target_oid: &str,
    ) -> Result<(), BranchPreviewError> {
        let branches = self.branches(repository)?;
        verify_branch(
            &branches,
            branch_full_name,
            expected_branch_oid,
            BranchPreviewError::BranchMoved,
        )?;
        verify_branch(
            &branches,
            target_full_name,
            expected_target_oid,
            BranchPreviewError::TargetMoved,
        )
    }

    fn commit_set(
        &self,
        repository: &Path,
        branch_oid: &str,
        target_oid: &str,
        first_parent: bool,
        skip: usize,
        max_count: usize,
    ) -> Result<HashSet<String>, BranchPreviewError> {
        if max_count == 0 {
            return Ok(HashSet::new());
        }
        let output = self.executor.execute_branch_preview(
            repository,
            BranchPreviewQuery::UniqueCommits {
                branch_oid: branch_oid.to_owned(),
                target_oid: target_oid.to_owned(),
                first_parent,
                skip,
                max_count,
            },
        )?;
        parse_oid_set(&output.stdout)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreviewCursor {
    branch_oid: String,
    history_offset: usize,
    // `rev-list branch ^target` is the history walk with base commits filtered out. Persisting
    // both filtered offsets lets every page request only its next bounded window instead of
    // materializing the complete unique history again.
    unique_offset: usize,
    first_parent_offset: usize,
}

impl PreviewCursor {
    fn new(branch_oid: &str) -> Self {
        Self {
            branch_oid: branch_oid.to_owned(),
            history_offset: 0,
            unique_offset: 0,
            first_parent_offset: 0,
        }
    }

    fn encode(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.branch_oid, self.history_offset, self.unique_offset, self.first_parent_offset
        )
    }
}

fn verify_branch(
    branches: &[app_domain::RepositoryBranch],
    full_name: &str,
    expected_oid: &str,
    moved_error: BranchPreviewError,
) -> Result<(), BranchPreviewError> {
    let branch = branches
        .iter()
        .find(|branch| branch.kind == RepositoryBranchKind::Local && branch.full_name == full_name);
    match branch {
        Some(branch) if branch.oid == expected_oid && branch.symbolic_target.is_none() => Ok(()),
        _ => Err(moved_error),
    }
}

fn validate_local_ref(value: &str) -> Result<(), BranchPreviewError> {
    let name = value
        .strip_prefix("refs/heads/")
        .filter(|name| !name.is_empty())
        .ok_or(BranchPreviewError::InvalidBranch)?;
    if name.starts_with('-')
        || name.contains("..")
        || name.contains("@{")
        || name.contains('\\')
        || name.chars().any(char::is_control)
    {
        return Err(BranchPreviewError::InvalidBranch);
    }
    Ok(())
}

fn validate_oid(value: &str) -> Result<(), BranchPreviewError> {
    if valid_oid(value) {
        Ok(())
    } else {
        Err(BranchPreviewError::InvalidObjectId)
    }
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_preview_cursor(
    cursor: &str,
    expected_branch_oid: &str,
) -> Result<PreviewCursor, BranchPreviewError> {
    let mut fields = cursor.split(':');
    let branch_oid = fields.next().ok_or(BranchPreviewError::InvalidCursor)?;
    let history_offset = parse_cursor_offset(fields.next())?;
    let unique_offset = parse_cursor_offset(fields.next())?;
    let first_parent_offset = parse_cursor_offset(fields.next())?;
    if fields.next().is_some() || !valid_oid(branch_oid) || branch_oid != expected_branch_oid {
        return Err(BranchPreviewError::InvalidCursor);
    }
    if unique_offset > history_offset || first_parent_offset > unique_offset {
        return Err(BranchPreviewError::InvalidCursor);
    }
    Ok(PreviewCursor {
        branch_oid: branch_oid.to_owned(),
        history_offset,
        unique_offset,
        first_parent_offset,
    })
}

fn parse_cursor_offset(value: Option<&str>) -> Result<usize, BranchPreviewError> {
    let value = value
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or(BranchPreviewError::InvalidCursor)?;
    value
        .parse()
        .ok()
        .filter(|offset| *offset <= MAX_PREVIEW_OFFSET)
        .ok_or(BranchPreviewError::InvalidCursor)
}

fn advance_cursor_offset(offset: usize, delta: usize) -> Result<usize, BranchPreviewError> {
    offset
        .checked_add(delta)
        .filter(|offset| *offset <= MAX_PREVIEW_OFFSET)
        .ok_or(BranchPreviewError::InvalidCursor)
}

fn parse_oid_set(output: &[u8]) -> Result<HashSet<String>, BranchPreviewError> {
    let text = std::str::from_utf8(output).map_err(|_| BranchPreviewError::InvalidObjectId)?;
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|oid| {
            validate_oid(oid)?;
            Ok(oid.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HistoryQuery, NavigationQuery};
    use std::{collections::VecDeque, sync::Mutex};

    struct MovingRefExecutor {
        navigation_outputs: Mutex<VecDeque<Vec<u8>>>,
        branch_oid: String,
    }

    impl NavigationGitExecutor for MovingRefExecutor {
        fn execute_navigation(
            &self,
            _repository: &Path,
            query: NavigationQuery,
        ) -> Result<GitOutput, GitRunError> {
            assert_eq!(query, NavigationQuery::Branches);
            Ok(GitOutput {
                stdout: self
                    .navigation_outputs
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("recorded navigation output"),
                stderr: Vec::new(),
            })
        }
    }

    impl HistoryGitExecutor for MovingRefExecutor {
        fn execute_history(
            &self,
            _repository: &Path,
            query: HistoryQuery,
        ) -> Result<GitOutput, GitRunError> {
            assert!(matches!(query, HistoryQuery::Page { .. }));
            Ok(GitOutput {
                stdout: format!(
                    "{}\0\0Preview\0preview@example.test\02026-07-22T12:00:00Z\0task\0\0",
                    self.branch_oid
                )
                .into_bytes(),
                stderr: Vec::new(),
            })
        }
    }

    impl BranchPreviewGitExecutor for MovingRefExecutor {
        fn execute_branch_preview(
            &self,
            _repository: &Path,
            _query: BranchPreviewQuery,
        ) -> Result<GitOutput, GitRunError> {
            Ok(GitOutput {
                stdout: format!("{}\n", self.branch_oid).into_bytes(),
                stderr: Vec::new(),
            })
        }
    }

    fn branch_records(branch_oid: &str, target_oid: &str) -> Vec<u8> {
        format!(
            "refs/heads/feature\0feature\0{branch_oid}\0\0\0\0\0\nrefs/heads/release\0release\0{target_oid}\0\0\0\0\0\n"
        )
        .into_bytes()
    }

    #[test]
    fn parses_only_complete_object_ids() {
        let oid = "a".repeat(40);
        assert_eq!(
            parse_oid_set(format!("{oid}\n").as_bytes()).unwrap().len(),
            1
        );
        assert!(parse_oid_set(b"main\n").is_err());
    }

    #[test]
    fn cursor_must_belong_to_exact_snapshot() {
        let oid = "b".repeat(40);
        assert_eq!(
            parse_preview_cursor(&format!("{oid}:250:175:90"), &oid).unwrap(),
            PreviewCursor {
                branch_oid: oid.clone(),
                history_offset: 250,
                unique_offset: 175,
                first_parent_offset: 90,
            }
        );
        assert!(parse_preview_cursor(&format!("{oid}:250:nope:90"), &oid).is_err());
        assert!(parse_preview_cursor(&format!("{oid}:250:251:90"), &oid).is_err());
        assert!(parse_preview_cursor(&format!("{oid}:250:175:176"), &oid).is_err());
        assert!(parse_preview_cursor(&format!("{oid}:250:175:90:1"), &oid).is_err());
        assert!(parse_preview_cursor(&format!("{oid}:250:175:90"), &"c".repeat(40)).is_err());
        assert!(
            parse_preview_cursor(&format!("{oid}:{}:175:90", MAX_PREVIEW_OFFSET + 1), &oid)
                .is_err()
        );
        assert!(
            parse_preview_cursor(
                &format!("{oid}:{}:{}:90", MAX_PREVIEW_OFFSET, MAX_PREVIEW_OFFSET + 1),
                &oid
            )
            .is_err()
        );
        assert!(
            parse_preview_cursor(
                &format!(
                    "{oid}:{}:{}:{}",
                    MAX_PREVIEW_OFFSET,
                    MAX_PREVIEW_OFFSET,
                    MAX_PREVIEW_OFFSET + 1
                ),
                &oid
            )
            .is_err()
        );
        assert_eq!(
            parse_preview_cursor(&format!("{oid}:{0}:{0}:{0}", MAX_PREVIEW_OFFSET), &oid)
                .unwrap()
                .history_offset,
            MAX_PREVIEW_OFFSET
        );
    }

    #[test]
    fn advancing_cursor_cannot_cross_the_preview_bound() {
        assert_eq!(
            advance_cursor_offset(MAX_PREVIEW_OFFSET - 1, 1).unwrap(),
            MAX_PREVIEW_OFFSET
        );
        assert!(advance_cursor_offset(MAX_PREVIEW_OFFSET, 1).is_err());
    }

    #[test]
    fn classification_query_is_bounded_to_the_requested_page() {
        let query = BranchPreviewQuery::UniqueCommits {
            branch_oid: "a".repeat(40),
            target_oid: "b".repeat(40),
            first_parent: true,
            skip: 75_000,
            max_count: 250,
        };
        let arguments = query
            .arguments()
            .into_iter()
            .map(|value| value.into_string().unwrap())
            .collect::<Vec<_>>();
        assert!(arguments.contains(&"--topo-order".to_owned()));
        assert!(arguments.contains(&"--first-parent".to_owned()));
        assert!(arguments.contains(&"--skip=75000".to_owned()));
        assert!(arguments.contains(&"--max-count=250".to_owned()));
        assert!(!arguments.contains(&"--max-count=50000".to_owned()));
    }

    #[test]
    fn rejects_results_when_the_branch_moves_during_classification() {
        let branch_oid = "a".repeat(40);
        let target_oid = "b".repeat(40);
        let moved_oid = "c".repeat(40);
        let runtime = RepositoryRuntime::new(MovingRefExecutor {
            navigation_outputs: Mutex::new(VecDeque::from([
                branch_records(&branch_oid, &target_oid),
                branch_records(&moved_oid, &target_oid),
            ])),
            branch_oid: branch_oid.clone(),
        });

        let error = runtime
            .branch_history_page(
                Path::new("/repo"),
                "refs/heads/feature",
                &branch_oid,
                "refs/heads/release",
                &target_oid,
                1,
                None,
            )
            .expect_err("branch movement during queries must invalidate the page");

        assert!(matches!(error, BranchPreviewError::BranchMoved));
    }

    #[test]
    fn local_ref_validation_rejects_revision_syntax() {
        assert!(validate_local_ref("refs/heads/feature/task").is_ok());
        assert!(validate_local_ref("refs/remotes/origin/main").is_err());
        assert!(validate_local_ref("refs/heads/main..other").is_err());
        assert!(validate_local_ref("refs/heads/main@{1}").is_err());
    }
}
