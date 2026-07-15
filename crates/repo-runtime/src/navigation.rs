use std::{path::Path, time::Duration};

use app_domain::{RepositoryBranch, RepositoryNavigation, RepositoryStash, RepositoryWorktree};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, NavigationParseError,
    parse_branch_records, parse_stash_records, parse_worktree_porcelain_z,
};
use thiserror::Error;

use super::RepositoryRuntime;

const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const SMALL_OUTPUT_LIMIT: usize = 64 * 1024;
const NAVIGATION_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;

const BRANCH_FORMAT: &str = "--format=%(refname)%00%(refname:short)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(symref)%00";
const STASH_FORMAT: &str = "--format=%H%x00%gd%x00%gs%x00%an%x00%aI%x00";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationQuery {
    Branches,
    Worktrees,
    StashPresence,
    Stashes,
}

impl NavigationQuery {
    fn arguments(self) -> Vec<&'static str> {
        match self {
            Self::Branches => vec!["for-each-ref", BRANCH_FORMAT, "refs/heads", "refs/remotes"],
            Self::Worktrees => vec!["worktree", "list", "--porcelain", "-z"],
            Self::StashPresence => vec![
                "for-each-ref",
                "--format=%(refname)",
                "--count=1",
                "refs/stash",
            ],
            Self::Stashes => vec!["log", "-g", STASH_FORMAT, "refs/stash"],
        }
    }

    fn stdout_limit(self) -> usize {
        match self {
            Self::StashPresence => SMALL_OUTPUT_LIMIT,
            Self::Branches | Self::Worktrees | Self::Stashes => NAVIGATION_OUTPUT_LIMIT,
        }
    }
}

pub trait NavigationGitExecutor: Send + Sync {
    fn execute_navigation(
        &self,
        repository: &Path,
        query: NavigationQuery,
    ) -> Result<GitOutput, GitRunError>;
}

impl NavigationGitExecutor for GitRunner {
    fn execute_navigation(
        &self,
        repository: &Path,
        query: NavigationQuery,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, query.arguments())
                .with_output_limits(query.stdout_limit(), STDERR_LIMIT)
                .with_timeout(QUERY_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum NavigationRuntimeError {
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    InvalidOutput(#[from] NavigationParseError),
}

impl<E> RepositoryRuntime<E>
where
    E: NavigationGitExecutor,
{
    pub fn navigation(
        &self,
        repository: &Path,
    ) -> Result<RepositoryNavigation, NavigationRuntimeError> {
        Ok(RepositoryNavigation {
            branches: self.branches(repository)?,
            worktrees: self.worktrees(repository)?,
            stashes: self.stashes(repository)?,
        })
    }

    pub fn branches(
        &self,
        repository: &Path,
    ) -> Result<Vec<RepositoryBranch>, NavigationRuntimeError> {
        let output = self
            .executor
            .execute_navigation(repository, NavigationQuery::Branches)?;
        Ok(parse_branch_records(&output.stdout)?)
    }

    pub fn worktrees(
        &self,
        repository: &Path,
    ) -> Result<Vec<RepositoryWorktree>, NavigationRuntimeError> {
        let output = self
            .executor
            .execute_navigation(repository, NavigationQuery::Worktrees)?;
        Ok(parse_worktree_porcelain_z(&output.stdout)?)
    }

    pub fn stashes(
        &self,
        repository: &Path,
    ) -> Result<Vec<RepositoryStash>, NavigationRuntimeError> {
        let presence = self
            .executor
            .execute_navigation(repository, NavigationQuery::StashPresence)?;
        if presence.stdout.is_empty() {
            return Ok(Vec::new());
        }

        let output = self
            .executor
            .execute_navigation(repository, NavigationQuery::Stashes)?;
        Ok(parse_stash_records(&output.stdout)?)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use git_core::GitExecutor;

    use super::*;

    struct RecordingExecutor {
        queries: Mutex<Vec<NavigationQuery>>,
        outputs: Mutex<VecDeque<Vec<u8>>>,
    }

    impl GitExecutor for RecordingExecutor {
        fn execute(
            &self,
            _repository: &Path,
            _arguments: &[&str],
        ) -> Result<GitOutput, GitRunError> {
            panic!("navigation tests must use the bounded navigation API")
        }
    }

    impl NavigationGitExecutor for RecordingExecutor {
        fn execute_navigation(
            &self,
            _repository: &Path,
            query: NavigationQuery,
        ) -> Result<GitOutput, GitRunError> {
            self.queries.lock().expect("queries lock").push(query);
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

    #[test]
    fn empty_stash_ref_does_not_run_log() {
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(VecDeque::from([Vec::new()])),
        });

        let stashes = runtime.stashes(Path::new("/repo")).expect("stash query");

        assert!(stashes.is_empty());
        assert_eq!(
            runtime
                .executor
                .queries
                .lock()
                .expect("queries lock")
                .as_slice(),
            &[NavigationQuery::StashPresence]
        );
    }

    #[test]
    fn navigation_runs_fixed_queries_and_combines_results() {
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            queries: Mutex::new(Vec::new()),
            outputs: Mutex::new(VecDeque::from([
                b"refs/heads/main\0main\0aaaa\0*\0\0\0\0\n".to_vec(),
                b"worktree /repo\0HEAD aaaa\0branch refs/heads/main\0\0".to_vec(),
                b"refs/stash\n".to_vec(),
                b"bbbb\0stash@{0}\0WIP\0Rafal\x002026-07-15T12:00:00+02:00\0\n".to_vec(),
            ])),
        });

        let navigation = runtime
            .navigation(Path::new("/repo;echo unsafe"))
            .expect("navigation query");

        assert_eq!(navigation.branches.len(), 1);
        assert_eq!(navigation.worktrees.len(), 1);
        assert_eq!(navigation.stashes.len(), 1);
        assert_eq!(
            runtime
                .executor
                .queries
                .lock()
                .expect("queries lock")
                .as_slice(),
            &[
                NavigationQuery::Branches,
                NavigationQuery::Worktrees,
                NavigationQuery::StashPresence,
                NavigationQuery::Stashes,
            ]
        );
    }
}
