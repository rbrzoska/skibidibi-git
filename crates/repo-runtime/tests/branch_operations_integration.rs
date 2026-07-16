use std::{fs, path::Path, process::Command};

use app_domain::{
    MergeBranchRequest, MergeBranchState, PullInactiveBranchRequest, StashCleanupState,
    StashRestoreState,
};
use repo_runtime::{BranchOperationError, RepositoryRuntime};
use tempfile::{TempDir, tempdir};

struct Fixture {
    _root: TempDir,
    _remote: std::path::PathBuf,
    local: std::path::PathBuf,
    peer: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempdir().unwrap();
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
        git(&seed, &["switch", "-c", "topic"]);
        git(&seed, &["switch", "main"]);
        git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&seed, &["push", "-u", "origin", "main", "topic"]);
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
        git(&local, &["branch", "--track", "topic", "origin/topic"]);
        git(&peer, &["switch", "topic"]);
        Self {
            _root: root,
            _remote: remote,
            local,
            peer,
        }
    }

    fn advance_remote_topic(&self) -> String {
        fs::write(self.peer.join("remote.txt"), "remote\n").unwrap();
        git(&self.peer, &["add", "--", "remote.txt"]);
        git(&self.peer, &["commit", "-m", "remote topic"]);
        git(&self.peer, &["push", "origin", "topic"]);
        oid(&self.peer, "HEAD")
    }
}

#[test]
fn merges_an_exact_source_into_the_exact_current_branch() {
    let fixture = Fixture::new();
    let source_name = "topic\u{a0}release";
    git(&fixture.local, &["switch", "-c", source_name]);
    fs::write(fixture.local.join("topic.txt"), "topic\n").unwrap();
    git(&fixture.local, &["add", "--", "topic.txt"]);
    git(&fixture.local, &["commit", "-m", "topic"]);
    let source = oid(&fixture.local, "HEAD");
    git(&fixture.local, &["switch", "main"]);
    let target = oid(&fixture.local, "HEAD");

    let result = RepositoryRuntime::default()
        .merge_branch(
            &fixture.local,
            &MergeBranchRequest {
                source_full_name: format!("refs/heads/{source_name}"),
                expected_source_oid: source.clone(),
                target_full_name: "refs/heads/main".to_owned(),
                expected_target_oid: target.clone(),
                auto_stash: None,
            },
        )
        .unwrap();

    assert_eq!(result.state, MergeBranchState::Succeeded);
    assert_eq!(result.head_before, target);
    assert_eq!(result.head_after.as_deref(), Some(source.as_str()));
    assert!(fixture.local.join("topic.txt").exists());
}

#[test]
fn merge_auto_stashes_and_restores_dirty_files() {
    let fixture = Fixture::new();
    git(&fixture.local, &["switch", "topic"]);
    fs::write(fixture.local.join("topic.txt"), "topic\n").unwrap();
    git(&fixture.local, &["add", "--", "topic.txt"]);
    git(&fixture.local, &["commit", "-m", "topic"]);
    let source = oid(&fixture.local, "HEAD");
    git(&fixture.local, &["switch", "main"]);
    fs::write(fixture.local.join("dirty.txt"), "dirty\n").unwrap();
    let result = RepositoryRuntime::default()
        .merge_branch(
            &fixture.local,
            &MergeBranchRequest {
                source_full_name: "refs/heads/topic".to_owned(),
                expected_source_oid: source,
                target_full_name: "refs/heads/main".to_owned(),
                expected_target_oid: oid(&fixture.local, "HEAD"),
                auto_stash: Some(app_domain::AutoStashOptions {
                    message: "WIP merge main".to_owned(),
                }),
            },
        )
        .unwrap();
    assert_eq!(result.state, MergeBranchState::Succeeded);
    assert_eq!(result.auto_stash.restore, StashRestoreState::Applied);
    assert_eq!(result.auto_stash.cleanup, StashCleanupState::Dropped);
    assert!(fixture.local.join("dirty.txt").exists());
}

#[test]
fn creates_a_non_fast_forward_merge_from_the_exact_target() {
    let fixture = Fixture::new();
    git(&fixture.local, &["switch", "topic"]);
    fs::write(fixture.local.join("topic.txt"), "topic\n").unwrap();
    git(&fixture.local, &["add", "--", "topic.txt"]);
    git(&fixture.local, &["commit", "-m", "topic"]);
    let source = oid(&fixture.local, "HEAD");
    git(&fixture.local, &["switch", "main"]);
    fs::write(fixture.local.join("main.txt"), "main\n").unwrap();
    git(&fixture.local, &["add", "--", "main.txt"]);
    git(&fixture.local, &["commit", "-m", "main"]);
    let target = oid(&fixture.local, "HEAD");

    let result = RepositoryRuntime::default()
        .merge_branch(
            &fixture.local,
            &MergeBranchRequest {
                source_full_name: "refs/heads/topic".to_owned(),
                expected_source_oid: source,
                target_full_name: "refs/heads/main".to_owned(),
                expected_target_oid: target.clone(),
                auto_stash: None,
            },
        )
        .unwrap();

    assert_eq!(result.state, MergeBranchState::Succeeded);
    assert_ne!(result.head_after.as_deref(), Some(target.as_str()));
    assert!(!result.mutation_may_have_occurred);
}

#[test]
fn merge_rejects_a_stale_source_without_moving_head() {
    let fixture = Fixture::new();
    let head = oid(&fixture.local, "HEAD");
    let error = RepositoryRuntime::default()
        .merge_branch(
            &fixture.local,
            &MergeBranchRequest {
                source_full_name: "refs/heads/topic".to_owned(),
                expected_source_oid: "0000000000000000000000000000000000000000".to_owned(),
                target_full_name: "refs/heads/main".to_owned(),
                expected_target_oid: head.clone(),
                auto_stash: None,
            },
        )
        .unwrap_err();
    assert!(matches!(error, BranchOperationError::StaleBranch));
    assert_eq!(oid(&fixture.local, "HEAD"), head);
}

#[test]
fn distinct_branches_at_the_same_oid_merge_as_an_up_to_date_success() {
    let fixture = Fixture::new();
    let head = oid(&fixture.local, "HEAD");
    let result = RepositoryRuntime::default()
        .merge_branch(
            &fixture.local,
            &MergeBranchRequest {
                source_full_name: "refs/heads/topic".to_owned(),
                expected_source_oid: head.clone(),
                target_full_name: "refs/heads/main".to_owned(),
                expected_target_oid: head.clone(),
                auto_stash: None,
            },
        )
        .unwrap();
    assert_eq!(result.state, MergeBranchState::Succeeded);
    assert_eq!(result.head_after.as_deref(), Some(head.as_str()));
}

#[test]
fn merge_conflict_returns_unmerged_status_for_the_conflict_resolver() {
    let fixture = Fixture::new();
    git(&fixture.local, &["switch", "topic"]);
    fs::write(fixture.local.join("base.txt"), "topic\n").unwrap();
    git(&fixture.local, &["add", "--", "base.txt"]);
    git(&fixture.local, &["commit", "-m", "topic conflict"]);
    let source = oid(&fixture.local, "HEAD");
    git(&fixture.local, &["switch", "main"]);
    fs::write(fixture.local.join("base.txt"), "main\n").unwrap();
    git(&fixture.local, &["add", "--", "base.txt"]);
    git(&fixture.local, &["commit", "-m", "main conflict"]);
    let target = oid(&fixture.local, "HEAD");
    let result = RepositoryRuntime::default()
        .merge_branch(
            &fixture.local,
            &MergeBranchRequest {
                source_full_name: "refs/heads/topic".to_owned(),
                expected_source_oid: source,
                target_full_name: "refs/heads/main".to_owned(),
                expected_target_oid: target,
                auto_stash: None,
            },
        )
        .unwrap();
    assert_eq!(result.state, MergeBranchState::Conflicted);
    assert!(result.status.unwrap().entries.iter().any(|entry| {
        entry.kind == app_domain::StatusEntryKind::Unmerged && entry.path == "base.txt"
    }));
}

#[test]
fn pulls_an_unchecked_out_branch_with_an_atomic_fast_forward() {
    let fixture = Fixture::new();
    let before = oid(&fixture.local, "refs/heads/topic");
    let remote = fixture.advance_remote_topic();
    let result = RepositoryRuntime::default()
        .pull_inactive_branch(
            &fixture.local,
            &PullInactiveBranchRequest {
                branch_full_name: "refs/heads/topic".to_owned(),
                expected_oid: before,
                expected_upstream: "origin/topic".to_owned(),
            },
        )
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.head_after, remote);
    assert_eq!(oid(&fixture.local, "refs/heads/topic"), remote);
}

#[test]
fn inactive_pull_rejects_divergence_without_moving_the_local_ref() {
    let fixture = Fixture::new();
    let linked = fixture._root.path().join("local-topic");
    git(
        &fixture.local,
        &["worktree", "add", linked.to_str().unwrap(), "topic"],
    );
    fs::write(linked.join("local.txt"), "local\n").unwrap();
    git(&linked, &["add", "--", "local.txt"]);
    git(&linked, &["commit", "-m", "local topic"]);
    let local_oid = oid(&linked, "HEAD");
    git(
        &fixture.local,
        &["worktree", "remove", linked.to_str().unwrap()],
    );
    fixture.advance_remote_topic();

    let error = RepositoryRuntime::default()
        .pull_inactive_branch(
            &fixture.local,
            &PullInactiveBranchRequest {
                branch_full_name: "refs/heads/topic".to_owned(),
                expected_oid: local_oid.clone(),
                expected_upstream: "origin/topic".to_owned(),
            },
        )
        .unwrap_err();
    assert!(matches!(error, BranchOperationError::NonFastForward));
    assert_eq!(oid(&fixture.local, "refs/heads/topic"), local_oid);
}

#[test]
fn pulls_a_clean_branch_inside_its_linked_worktree_but_rejects_it_when_dirty() {
    let fixture = Fixture::new();
    let linked = fixture._root.path().join("linked-topic");
    git(
        &fixture.local,
        &["worktree", "add", linked.to_str().unwrap(), "topic"],
    );
    let before = oid(&fixture.local, "refs/heads/topic");
    let remote = fixture.advance_remote_topic();
    let runtime = RepositoryRuntime::default();
    runtime
        .pull_inactive_branch(
            &fixture.local,
            &PullInactiveBranchRequest {
                branch_full_name: "refs/heads/topic".to_owned(),
                expected_oid: before,
                expected_upstream: "origin/topic".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(oid(&linked, "HEAD"), remote);

    fs::write(linked.join("dirty.txt"), "dirty\n").unwrap();
    let error = runtime
        .pull_inactive_branch(
            &fixture.local,
            &PullInactiveBranchRequest {
                branch_full_name: "refs/heads/topic".to_owned(),
                expected_oid: remote,
                expected_upstream: "origin/topic".to_owned(),
            },
        )
        .unwrap_err();
    assert!(matches!(error, BranchOperationError::DirtyWorkingTree));
}

#[test]
fn dirty_state_scan_isolates_worktrees_and_counts_changes() {
    let fixture = Fixture::new();
    let linked = fixture._root.path().join("linked-topic");
    git(
        &fixture.local,
        &["worktree", "add", linked.to_str().unwrap(), "topic"],
    );
    fs::write(linked.join("dirty.txt"), "dirty\n").unwrap();
    let states = RepositoryRuntime::default()
        .worktree_dirty_states(&fixture.local)
        .unwrap();
    let topic = states
        .iter()
        .find(|state| state.branch_full_name.as_deref() == Some("refs/heads/topic"))
        .unwrap();
    assert!(topic.dirty);
    assert_eq!(topic.change_count, 1);
    assert!(topic.error_message.is_none());
}

fn configure(repository: &Path) {
    git(repository, &["config", "user.name", "Skibidibi Test"]);
    git(
        repository,
        &["config", "user.email", "test@example.invalid"],
    );
}

fn oid(repository: &Path, revision: &str) -> String {
    String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["rev-parse", revision])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned()
}

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
