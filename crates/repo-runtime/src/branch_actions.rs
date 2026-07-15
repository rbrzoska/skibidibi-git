use std::{path::Path, time::Duration};

use app_domain::{RepositoryBranch, RepositoryBranchKind, SwitchBranchRequest, SwitchBranchResult};
use git_core::{
    GitInvocation, GitInvocationPolicy, GitOutput, GitRunError, GitRunner, NavigationParseError,
    parse_branch_records,
};
use thiserror::Error;

use super::RepositoryRuntime;

const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const BRANCH_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const ACTION_STDOUT_LIMIT: usize = 256 * 1024;
const STDERR_LIMIT: usize = 256 * 1024;
const BRANCH_FORMAT: &str = "--format=%(refname)%00%(refname:short)%00%(objectname)%00%(HEAD)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(symref)%00";

/// Narrow executor boundary that keeps branch discovery read-only and switching explicitly
/// mutating. Implementations must pass argv directly to Git without a shell.
pub trait BranchActionGitExecutor: Send + Sync {
    fn query_switchable_branches(&self, repository: &Path) -> Result<GitOutput, GitRunError>;

    fn switch_existing_local_branch(
        &self,
        repository: &Path,
        branch_name: &str,
    ) -> Result<GitOutput, GitRunError>;
}

impl BranchActionGitExecutor for GitRunner {
    fn query_switchable_branches(&self, repository: &Path) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::ReadOnly,
                ["for-each-ref", BRANCH_FORMAT, "refs/heads", "refs/remotes"],
            )
            .with_output_limits(BRANCH_OUTPUT_LIMIT, STDERR_LIMIT)
            .with_timeout(QUERY_TIMEOUT),
        )
    }

    fn switch_existing_local_branch(
        &self,
        repository: &Path,
        branch_name: &str,
    ) -> Result<GitOutput, GitRunError> {
        self.run(
            repository,
            GitInvocation::new(
                GitInvocationPolicy::Mutating,
                ["switch", "--no-guess", "--", branch_name],
            )
            .with_output_limits(ACTION_STDOUT_LIMIT, STDERR_LIMIT)
            .with_timeout(ACTION_TIMEOUT),
        )
    }
}

#[derive(Debug, Error)]
pub enum BranchSwitchError {
    #[error("branch ref must be a valid fully-qualified local ref under refs/heads")]
    InvalidLocalRef,
    #[error("remote branch refs cannot be checked out by this operation")]
    RemoteBranchNotAllowed,
    #[error("local branch does not exist")]
    BranchNotFound,
    #[error("symbolic branch refs cannot be checked out by this operation")]
    SymbolicBranchNotAllowed,
    #[error(transparent)]
    Git(#[from] GitRunError),
    #[error(transparent)]
    InvalidOutput(#[from] NavigationParseError),
}

impl<E> RepositoryRuntime<E>
where
    E: BranchActionGitExecutor,
{
    /// Switches only to a freshly-discovered, exact local branch. It never forces checkout,
    /// discards changes, contacts a remote, or stashes implicitly.
    pub fn switch_branch(
        &self,
        repository: &Path,
        request: &SwitchBranchRequest,
    ) -> Result<SwitchBranchResult, BranchSwitchError> {
        validate_requested_ref(&request.full_name)?;

        let output = self.executor.query_switchable_branches(repository)?;
        let branches = parse_branch_records(&output.stdout)?;
        let branch = branches
            .iter()
            .find(|branch| branch.full_name == request.full_name)
            .ok_or(BranchSwitchError::BranchNotFound)?;

        if branch.kind != RepositoryBranchKind::Local {
            return Err(BranchSwitchError::RemoteBranchNotAllowed);
        }
        if branch.symbolic_target.is_some() {
            return Err(BranchSwitchError::SymbolicBranchNotAllowed);
        }

        let result = result_from_branch(branch, !branch.current);
        if branch.current {
            return Ok(result);
        }

        self.executor
            .switch_existing_local_branch(repository, &branch.name)?;
        Ok(result)
    }
}

fn validate_requested_ref(full_name: &str) -> Result<(), BranchSwitchError> {
    if full_name.starts_with("refs/remotes/") {
        return Err(BranchSwitchError::RemoteBranchNotAllowed);
    }

    let Some(name) = full_name.strip_prefix("refs/heads/") else {
        return Err(BranchSwitchError::InvalidLocalRef);
    };
    if !is_valid_branch_name(name) {
        return Err(BranchSwitchError::InvalidLocalRef);
    }
    Ok(())
}

/// Mirrors the safety-relevant constraints from `git check-ref-format --branch` locally so no
/// attacker-controlled validation command has to run before the exact ref lookup.
fn is_valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name != "@"
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with('.')
        && !name.contains("..")
        && !name.contains("@{")
        && !name.contains("//")
        && !name
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
        && !name.bytes().any(|byte| {
            matches!(
                byte,
                0x00..=0x20 | 0x7f | b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\'
            )
        })
}

fn result_from_branch(branch: &RepositoryBranch, changed: bool) -> SwitchBranchResult {
    SwitchBranchResult {
        full_name: branch.full_name.clone(),
        name: branch.name.clone(),
        head: branch.oid.clone(),
        changed,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use git_core::GitExecutor;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum RecordedCall {
        Query,
        Switch(String),
    }

    struct RecordingExecutor {
        branch_output: Vec<u8>,
        switch_error: Option<String>,
        calls: Mutex<Vec<RecordedCall>>,
    }

    impl GitExecutor for RecordingExecutor {
        fn execute(
            &self,
            _repository: &Path,
            _arguments: &[&str],
        ) -> Result<GitOutput, GitRunError> {
            panic!("branch action tests must use the explicit policy API")
        }
    }

    impl BranchActionGitExecutor for RecordingExecutor {
        fn query_switchable_branches(&self, _repository: &Path) -> Result<GitOutput, GitRunError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(RecordedCall::Query);
            Ok(GitOutput {
                stdout: self.branch_output.clone(),
                stderr: Vec::new(),
            })
        }

        fn switch_existing_local_branch(
            &self,
            _repository: &Path,
            branch_name: &str,
        ) -> Result<GitOutput, GitRunError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(RecordedCall::Switch(branch_name.to_owned()));
            if let Some(stderr) = &self.switch_error {
                return Err(GitRunError::Unsuccessful {
                    code: Some(1),
                    stderr: stderr.clone(),
                });
            }
            Ok(GitOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    fn runtime(branch_output: &[u8]) -> RepositoryRuntime<RecordingExecutor> {
        RepositoryRuntime::new(RecordingExecutor {
            branch_output: branch_output.to_vec(),
            switch_error: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn request(full_name: &str) -> SwitchBranchRequest {
        SwitchBranchRequest {
            full_name: full_name.to_owned(),
        }
    }

    #[test]
    fn switches_an_exact_existing_local_ref_using_its_short_name() {
        let runtime = runtime(
            b"refs/heads/main\0main\0aaaa\0*\0\0\0\0\nrefs/heads/rb/safe\0rb/safe\0bbbb\0 \0\0\0\0\n",
        );

        let result = runtime
            .switch_branch(Path::new("/repo"), &request("refs/heads/rb/safe"))
            .expect("switch succeeds");

        assert_eq!(result.full_name, "refs/heads/rb/safe");
        assert_eq!(result.name, "rb/safe");
        assert_eq!(result.head, "bbbb");
        assert!(result.changed);
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[
                RecordedCall::Query,
                RecordedCall::Switch("rb/safe".to_owned())
            ]
        );
    }

    #[test]
    fn current_branch_is_a_no_op_after_fresh_discovery() {
        let runtime = runtime(b"refs/heads/main\0main\0aaaa\0*\0\0\0\0\n");

        let result = runtime
            .switch_branch(Path::new("/repo"), &request("refs/heads/main"))
            .expect("current branch is accepted");

        assert!(!result.changed);
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[RecordedCall::Query]
        );
    }

    #[test]
    fn rejects_remote_ref_without_querying_git() {
        let runtime = runtime(b"refs/remotes/origin/main\0origin/main\0aaaa\0 \0\0\0\0\n");

        let error = runtime
            .switch_branch(Path::new("/repo"), &request("refs/remotes/origin/main"))
            .expect_err("remote ref is rejected");

        assert!(matches!(error, BranchSwitchError::RemoteBranchNotAllowed));
        assert!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .is_empty()
        );
    }

    #[test]
    fn rejects_malicious_or_ambiguous_refs_without_querying_git() {
        for full_name in [
            "--discard-changes",
            "refs/heads/-discard-changes",
            "refs/heads/safe\nbranch",
            "refs/heads/topic..other",
            "refs/heads/topic.lock",
        ] {
            let runtime = runtime(&[]);
            let error = runtime
                .switch_branch(Path::new("/repo"), &request(full_name))
                .expect_err("unsafe ref is rejected");
            assert!(matches!(error, BranchSwitchError::InvalidLocalRef));
            assert!(
                runtime
                    .executor
                    .calls
                    .lock()
                    .expect("calls lock")
                    .is_empty()
            );
        }
    }

    #[test]
    fn accepts_unicode_whitespace_that_git_allows_in_a_local_ref() {
        let full_name = "refs/heads/feature/\u{00a0}topic";
        let branch_name = "feature/\u{00a0}topic";
        let output = format!("{full_name}\0{branch_name}\0bbbb\0 \0\0\0\0\n");
        let runtime = runtime(output.as_bytes());

        let result = runtime
            .switch_branch(Path::new("/repo"), &request(full_name))
            .expect("Git-compatible Unicode branch is accepted");

        assert_eq!(result.full_name, full_name);
        assert_eq!(result.name, branch_name);
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[
                RecordedCall::Query,
                RecordedCall::Switch(branch_name.to_owned()),
            ]
        );
    }

    #[test]
    fn rejects_missing_and_symbolic_branches_without_switching() {
        let missing = runtime(b"refs/heads/main\0main\0aaaa\0*\0\0\0\0\n");
        assert!(matches!(
            missing.switch_branch(Path::new("/repo"), &request("refs/heads/other")),
            Err(BranchSwitchError::BranchNotFound)
        ));

        let symbolic = runtime(b"refs/heads/alias\0alias\0aaaa\0 \0\0\0refs/heads/main\0\n");
        assert!(matches!(
            symbolic.switch_branch(Path::new("/repo"), &request("refs/heads/alias")),
            Err(BranchSwitchError::SymbolicBranchNotAllowed)
        ));
        assert_eq!(
            symbolic
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[RecordedCall::Query]
        );
    }

    #[test]
    fn propagates_dirty_worktree_switch_failure_without_retry_or_force() {
        let runtime = RepositoryRuntime::new(RecordingExecutor {
            branch_output: b"refs/heads/feature\0feature\0bbbb\0 \0\0\0\0\n".to_vec(),
            switch_error: Some(
                "Your local changes to the following files would be overwritten".to_owned(),
            ),
            calls: Mutex::new(Vec::new()),
        });

        let error = runtime
            .switch_branch(Path::new("/repo"), &request("refs/heads/feature"))
            .expect_err("dirty switch fails");

        assert!(matches!(
            error,
            BranchSwitchError::Git(GitRunError::Unsuccessful { code: Some(1), stderr })
                if stderr.contains("would be overwritten")
        ));
        assert_eq!(
            runtime
                .executor
                .calls
                .lock()
                .expect("calls lock")
                .as_slice(),
            &[
                RecordedCall::Query,
                RecordedCall::Switch("feature".to_owned())
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn git_runner_uses_exact_bounded_mutating_argv() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let directory = tempfile::tempdir().expect("temp directory");
        let script = directory.path().join("fake-git");
        fs::write(&script, "#!/bin/sh\nprintf '%s\\0' \"$@\"\n").expect("write script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
            .expect("make script executable");
        let runner = GitRunner::new(script);

        let output = runner
            .switch_existing_local_branch(Path::new("/repo"), "rb/safe;echo nope")
            .expect("fake command succeeds");

        let arguments = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty())
            .map(|value| String::from_utf8(value.to_vec()).expect("utf8 argument"))
            .collect::<Vec<_>>();
        assert_eq!(
            arguments,
            [
                "--no-pager",
                "-c",
                "core.fsmonitor=false",
                "-C",
                "/repo",
                "switch",
                "--no-guess",
                "--",
                "rb/safe;echo nope",
            ]
        );
    }

    #[test]
    fn switches_a_real_git_branch_containing_non_breaking_space() {
        use std::{fs, process::Command};

        let directory = tempfile::tempdir().expect("temp directory");
        let repository = directory.path().join("repository");
        fs::create_dir(&repository).expect("create repository");

        let git = |arguments: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&repository)
                .args(arguments)
                .output()
                .expect("run git")
        };
        assert!(git(&["init", "-q"]).status.success());
        assert!(
            git(&["config", "user.email", "test@example.com"])
                .status
                .success()
        );
        assert!(git(&["config", "user.name", "Test User"]).status.success());
        fs::write(repository.join("tracked.txt"), "content\n").expect("write tracked file");
        assert!(git(&["add", "tracked.txt"]).status.success());
        assert!(git(&["commit", "-qm", "initial"]).status.success());

        let branch_name = "feature/\u{00a0}topic";
        let full_name = format!("refs/heads/{branch_name}");
        assert!(git(&["branch", branch_name]).status.success());

        let runtime = RepositoryRuntime::new(GitRunner::default());
        let result = runtime
            .switch_branch(&repository, &request(&full_name))
            .expect("switch Unicode branch");

        assert!(result.changed);
        assert_eq!(result.full_name, full_name);
        assert_eq!(result.name, branch_name);
        assert_eq!(
            String::from_utf8(git(&["symbolic-ref", "--short", "HEAD"]).stdout)
                .expect("UTF-8 HEAD")
                .trim(),
            branch_name
        );
    }
}
