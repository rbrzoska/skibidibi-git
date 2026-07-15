use std::{fs, path::Path, process::Command};

use app_domain::{
    AutoStashOptions, PullOperationState, PullRequest, PullStrategy, PushReadiness, PushRequest,
    PushTarget, RepositoryStatePrecondition, SetUpstreamRequest, StashCleanupState,
    StashRestoreState,
};
use repo_runtime::{NetworkOperationError, RepositoryRuntime};
use tempfile::{TempDir, tempdir};

struct Fixture {
    _root: TempDir,
    _remote: std::path::PathBuf,
    local: std::path::PathBuf,
    peer: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempdir().expect("temp root");
        let remote = root.path().join("remote.git");
        let seed = root.path().join("seed");
        let local = root.path().join("local");
        let peer = root.path().join("peer");
        git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
        git(
            root.path(),
            &["init", "--initial-branch=main", seed.to_str().unwrap()],
        );
        configure(&seed);
        fs::write(seed.join("base.txt"), "base\n").unwrap();
        git(&seed, &["add", "--", "base.txt"]);
        git(&seed, &["commit", "-m", "base"]);
        git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&seed, &["push", "-u", "origin", "main"]);
        git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(
            root.path(),
            &["clone", remote.to_str().unwrap(), local.to_str().unwrap()],
        );
        git(
            root.path(),
            &["clone", remote.to_str().unwrap(), peer.to_str().unwrap()],
        );
        configure(&local);
        configure(&peer);
        Self {
            _root: root,
            _remote: remote,
            local,
            peer,
        }
    }

    fn peer_commit(&self, path: &str, contents: &str, message: &str) {
        git(&self.peer, &["pull", "--ff-only", "origin", "main"]);
        fs::write(self.peer.join(path), contents).unwrap();
        git(&self.peer, &["add", "--", path]);
        git(&self.peer, &["commit", "-m", message]);
        git(&self.peer, &["push", "origin", "main"]);
    }
}

#[test]
fn ff_only_pull_fetches_exact_upstream_and_fast_forwards() {
    let fixture = Fixture::new();
    fixture.peer_commit("remote.txt", "remote\n", "remote");
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let result = runtime
        .pull_repository(
            &fixture.local,
            &PullRequest {
                strategy: PullStrategy::FfOnly,
                auto_stash: None,
                precondition: precondition(&before),
            },
        )
        .unwrap();
    assert_eq!(result.state, PullOperationState::Succeeded);
    assert!(fixture.local.join("remote.txt").exists());
    assert_ne!(
        result.head_after.as_deref(),
        Some(result.head_before.as_str())
    );
}

#[test]
fn ff_pull_auto_stashes_and_restores_tracked_and_untracked_changes() {
    let fixture = Fixture::new();
    fixture.peer_commit("remote.txt", "remote\n", "remote");
    fs::write(fixture.local.join("base.txt"), "dirty\n").unwrap();
    fs::write(fixture.local.join("untracked.txt"), "untracked\n").unwrap();
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let result = runtime
        .pull_repository(
            &fixture.local,
            &PullRequest {
                strategy: PullStrategy::FfOnly,
                auto_stash: Some(AutoStashOptions {
                    message: "WIP pull main".to_owned(),
                }),
                precondition: precondition(&before),
            },
        )
        .unwrap();
    assert_eq!(result.state, PullOperationState::Succeeded);
    assert_eq!(result.auto_stash.restore, StashRestoreState::Applied);
    assert_eq!(result.auto_stash.cleanup, StashCleanupState::Dropped);
    assert_eq!(
        fs::read_to_string(fixture.local.join("base.txt")).unwrap(),
        "dirty\n"
    );
    assert!(fixture.local.join("untracked.txt").exists());
    assert!(
        git_output(&fixture.local, &["stash", "list"])
            .trim()
            .is_empty()
    );
}

#[test]
fn pull_reports_auto_stash_restore_conflict_and_retains_recovery_stash() {
    let fixture = Fixture::new();
    fixture.peer_commit("base.txt", "remote change\n", "remote change");
    fs::write(fixture.local.join("base.txt"), "local dirty change\n").unwrap();
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let result = runtime
        .pull_repository(
            &fixture.local,
            &PullRequest {
                strategy: PullStrategy::FfOnly,
                auto_stash: Some(AutoStashOptions {
                    message: "WIP conflict main".to_owned(),
                }),
                precondition: precondition(&before),
            },
        )
        .unwrap();

    assert_eq!(result.state, PullOperationState::Conflicted);
    assert_eq!(result.auto_stash.restore, StashRestoreState::Conflicted);
    assert_eq!(result.auto_stash.cleanup, StashCleanupState::Retained);
    assert!(result.auto_stash.stash.is_some());
    assert!(
        !git_output(&fixture.local, &["stash", "list"])
            .trim()
            .is_empty()
    );
    assert!(
        result
            .status
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.kind == app_domain::StatusEntryKind::Unmerged)
    );
}

#[test]
fn rebase_pull_replays_local_commit_over_remote() {
    let fixture = Fixture::new();
    fs::write(fixture.local.join("local.txt"), "local\n").unwrap();
    git(&fixture.local, &["add", "--", "local.txt"]);
    git(&fixture.local, &["commit", "-m", "local"]);
    fixture.peer_commit("remote.txt", "remote\n", "remote");
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let result = runtime
        .pull_repository(
            &fixture.local,
            &PullRequest {
                strategy: PullStrategy::Rebase,
                auto_stash: None,
                precondition: precondition(&before),
            },
        )
        .unwrap();
    assert_eq!(result.state, PullOperationState::Succeeded);
    assert_eq!(result.status.unwrap().branch.ahead, 1);
    assert_eq!(
        git_output(&fixture.local, &["show", "-s", "--format=%P", "HEAD"])
            .split_whitespace()
            .count(),
        1
    );
}

#[test]
fn ff_if_possible_creates_a_merge_for_diverged_history() {
    let fixture = Fixture::new();
    fs::write(fixture.local.join("local.txt"), "local\n").unwrap();
    git(&fixture.local, &["add", "--", "local.txt"]);
    git(&fixture.local, &["commit", "-m", "local"]);
    fixture.peer_commit("remote.txt", "remote\n", "remote");
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let result = runtime
        .pull_repository(
            &fixture.local,
            &PullRequest {
                strategy: PullStrategy::FfIfPossible,
                auto_stash: None,
                precondition: precondition(&before),
            },
        )
        .unwrap();
    assert_eq!(result.state, PullOperationState::Succeeded);
    assert_eq!(
        git_output(&fixture.local, &["show", "-s", "--format=%P", "HEAD"])
            .split_whitespace()
            .count(),
        2
    );
}

#[test]
fn ff_if_possible_surfaces_merge_conflicts_for_the_resolver() {
    let fixture = Fixture::new();
    fs::write(fixture.local.join("base.txt"), "local commit\n").unwrap();
    git(&fixture.local, &["add", "--", "base.txt"]);
    git(&fixture.local, &["commit", "-m", "local conflict"]);
    fixture.peer_commit("base.txt", "remote commit\n", "remote conflict");
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let result = runtime
        .pull_repository(
            &fixture.local,
            &PullRequest {
                strategy: PullStrategy::FfIfPossible,
                auto_stash: None,
                precondition: precondition(&before),
            },
        )
        .unwrap();

    assert_eq!(result.state, PullOperationState::Conflicted);
    assert!(result.error_message.is_some());
    assert!(
        result
            .status
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.kind == app_domain::StatusEntryKind::Unmerged)
    );
}

#[test]
fn push_allows_only_ahead_and_blocks_behind_or_diverged() {
    let fixture = Fixture::new();
    fs::write(fixture.local.join("local.txt"), "local\n").unwrap();
    git(&fixture.local, &["add", "--", "local.txt"]);
    git(&fixture.local, &["commit", "-m", "local"]);
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let result = runtime
        .push_repository(
            &fixture.local,
            &PushRequest {
                target: PushTarget::Configured {
                    expected_upstream: "origin/main".to_owned(),
                },
                precondition: precondition(&before),
            },
        )
        .unwrap();
    assert!(result.pushed);
    assert_eq!(result.analysis.readiness, PushReadiness::UpToDate);

    fixture.peer_commit("peer.txt", "peer\n", "peer");
    let stale = runtime.status(&fixture.local).unwrap();
    let error = runtime
        .push_repository(
            &fixture.local,
            &PushRequest {
                target: PushTarget::Configured {
                    expected_upstream: "origin/main".to_owned(),
                },
                precondition: precondition(&stale),
            },
        )
        .unwrap_err();
    assert!(matches!(error, NetworkOperationError::PushBlocked));
}

#[test]
fn sets_upstream_only_to_fresh_exact_remote_tracking_ref() {
    let fixture = Fixture::new();
    git(&fixture.local, &["branch", "--unset-upstream"]);
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    let remote_oid = git_output(&fixture.local, &["rev-parse", "refs/remotes/origin/main"])
        .trim()
        .to_owned();
    let result = runtime
        .set_repository_upstream(
            &fixture.local,
            &SetUpstreamRequest {
                remote_full_name: "refs/remotes/origin/main".to_owned(),
                expected_oid: remote_oid,
                precondition: precondition(&before),
            },
        )
        .unwrap();
    assert_eq!(result.upstream, "origin/main");
    assert_eq!(
        result.status.branch.upstream.as_deref(),
        Some("origin/main")
    );
}

#[test]
fn push_can_publish_current_head_and_set_upstream_without_force() {
    let fixture = Fixture::new();
    git(&fixture.local, &["branch", "--unset-upstream"]);
    fs::write(fixture.local.join("published.txt"), "published\n").unwrap();
    git(&fixture.local, &["add", "--", "published.txt"]);
    git(&fixture.local, &["commit", "-m", "publish"]);
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(&fixture.local).unwrap();
    assert_eq!(
        runtime.push_analysis(&fixture.local).unwrap().readiness,
        PushReadiness::NoUpstream
    );

    let result = runtime
        .push_repository(
            &fixture.local,
            &PushRequest {
                target: PushTarget::SetUpstream {
                    remote: "origin".to_owned(),
                    remote_branch: "feature/published".to_owned(),
                },
                precondition: precondition(&before),
            },
        )
        .unwrap();

    assert!(result.pushed);
    assert_eq!(
        result.status.branch.upstream.as_deref(),
        Some("origin/feature/published")
    );
    assert_eq!(
        git_output(
            &fixture.local,
            &["rev-parse", "refs/remotes/origin/feature/published"]
        )
        .trim(),
        result.status.branch.oid.as_deref().unwrap()
    );
}

fn precondition(status: &app_domain::RepositoryStatus) -> RepositoryStatePrecondition {
    RepositoryStatePrecondition {
        expected_head: status.branch.oid.clone(),
        expected_head_name: status.branch.head.clone(),
        expected_detached: status.branch.detached,
        expected_unborn: status.branch.unborn,
        expected_index_fingerprint: status.index_fingerprint.clone(),
        expected_worktree_fingerprint: status.worktree_fingerprint.clone(),
    }
}

fn configure(repository: &Path) {
    git(repository, &["config", "user.name", "Network Test"]);
    git(
        repository,
        &["config", "user.email", "network@example.test"],
    );
}

fn git(repository: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
