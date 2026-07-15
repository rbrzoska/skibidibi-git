use std::{fs, path::Path, process::Command};

use app_domain::{StashCleanupState, StashRestoreState, SwitchBranchRequest};
use repo_runtime::{BranchSwitchError, RepositoryRuntime};
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should be installed for integration tests");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should be installed for integration tests");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Git output is UTF-8")
}

fn request(repository: &Path, full_name: &str) -> SwitchBranchRequest {
    SwitchBranchRequest {
        full_name: full_name.to_owned(),
        expected_oid: git_output(repository, &["rev-parse", full_name])
            .trim()
            .to_owned(),
        stash_on_dirty: false,
        stash_message: None,
    }
}

#[test]
fn switches_existing_local_branch_and_preserves_conflicting_dirty_changes() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "--initial-branch=main"]);
    git(repository, &["config", "user.name", "Switch Test"]);
    git(repository, &["config", "user.email", "switch@example.test"]);

    fs::write(repository.join("tracked.txt"), "base\n").expect("write base file");
    git(repository, &["add", "--", "tracked.txt"]);
    git(repository, &["commit", "-m", "base"]);
    git(repository, &["branch", "feature/safe"]);

    fs::write(repository.join("tracked.txt"), "main version\n").expect("write main version");
    git(repository, &["commit", "-am", "main change"]);
    git(repository, &["switch", "feature/safe"]);
    fs::write(repository.join("tracked.txt"), "dirty feature version\n")
        .expect("write dirty feature version");

    let runtime = RepositoryRuntime::default();
    let error = runtime
        .switch_branch(repository, &request(repository, "refs/heads/main"))
        .expect_err("Git must reject overwriting the dirty file");
    assert!(matches!(error, BranchSwitchError::DirtyWorktree));
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).expect("read preserved dirty file"),
        "dirty feature version\n"
    );

    git(repository, &["restore", "--", "tracked.txt"]);
    let result = runtime
        .switch_branch(repository, &request(repository, "refs/heads/main"))
        .expect("clean branch switch succeeds");
    assert!(result.changed);
    assert_eq!(result.full_name, "refs/heads/main");
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).expect("read main version"),
        "main version\n"
    );
}

#[test]
fn explicitly_stashes_tracked_and_untracked_changes_before_switching() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "--initial-branch=main"]);
    git(repository, &["config", "user.name", "Switch Test"]);
    git(repository, &["config", "user.email", "switch@example.test"]);
    fs::write(repository.join("tracked.txt"), "base\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    git(repository, &["commit", "-m", "base"]);
    git(repository, &["branch", "feature"]);
    git(repository, &["switch", "feature"]);
    fs::write(repository.join("tracked.txt"), "staged\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    fs::write(repository.join("tracked.txt"), "unstaged\n").unwrap();
    fs::write(repository.join("untracked.txt"), "untracked\n").unwrap();

    let runtime = RepositoryRuntime::default();
    let result = runtime
        .switch_branch(
            repository,
            &SwitchBranchRequest {
                full_name: "refs/heads/main".to_owned(),
                expected_oid: git_output(repository, &["rev-parse", "refs/heads/main"])
                    .trim()
                    .to_owned(),
                stash_on_dirty: true,
                stash_message: Some("WIP 2026-07-15T15:30:00 feature".to_owned()),
            },
        )
        .expect("stash and switch");

    assert!(result.changed);
    assert!(result.stash_created);
    assert!(result.operation_succeeded);
    assert_eq!(
        result.auto_stash.restore,
        StashRestoreState::Applied,
        "{:?}",
        result.auto_stash
    );
    assert_eq!(result.auto_stash.cleanup, StashCleanupState::Dropped);
    assert!(git_output(repository, &["stash", "list", "--format=%gs"]).is_empty());
    assert!(repository.join("untracked.txt").exists());
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "unstaged\n"
    );
    assert!(git_output(repository, &["diff", "--cached", "--", "tracked.txt"]).contains("+staged"));
    assert!(git_output(repository, &["diff", "--", "tracked.txt"]).contains("+unstaged"));
}

#[test]
fn auto_stash_restore_conflict_keeps_target_branch_and_stash() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "--initial-branch=main"]);
    git(repository, &["config", "user.name", "Switch Test"]);
    git(repository, &["config", "user.email", "switch@example.test"]);
    fs::write(repository.join("tracked.txt"), "base\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    git(repository, &["commit", "-m", "base"]);
    git(repository, &["branch", "feature"]);
    fs::write(repository.join("tracked.txt"), "main side\n").unwrap();
    git(repository, &["commit", "-am", "main side"]);
    git(repository, &["switch", "feature"]);
    fs::write(repository.join("tracked.txt"), "feature side\n").unwrap();

    let result = RepositoryRuntime::default()
        .switch_branch(
            repository,
            &SwitchBranchRequest {
                full_name: "refs/heads/main".to_owned(),
                expected_oid: git_output(repository, &["rev-parse", "refs/heads/main"])
                    .trim()
                    .to_owned(),
                stash_on_dirty: true,
                stash_message: Some("WIP 2026-07-15T15:31:00 feature".to_owned()),
            },
        )
        .expect("restore conflict is a structured successful switch");

    assert!(result.operation_succeeded);
    assert!(result.changed);
    assert_eq!(result.auto_stash.restore, StashRestoreState::Conflicted);
    assert_eq!(result.auto_stash.cleanup, StashCleanupState::Retained);
    assert_eq!(
        git_output(repository, &["branch", "--show-current"]).trim(),
        "main"
    );
    assert!(!git_output(repository, &["stash", "list", "--format=%H"]).is_empty());
    assert!(
        RepositoryRuntime::default()
            .status(repository)
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.kind == app_domain::StatusEntryKind::Unmerged)
    );
}

#[test]
fn failed_safe_switch_restores_changes_on_the_source_and_drops_auto_stash() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "--initial-branch=main"]);
    git(repository, &["config", "user.name", "Switch Test"]);
    git(repository, &["config", "user.email", "switch@example.test"]);
    fs::write(repository.join("tracked.txt"), "base\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    git(repository, &["commit", "-m", "base"]);
    git(repository, &["branch", "target"]);
    let linked_directory = tempdir().expect("linked worktree parent");
    let linked = linked_directory.path().join("linked-target");
    git(
        repository,
        &["worktree", "add", linked.to_str().unwrap(), "target"],
    );
    fs::write(repository.join("tracked.txt"), "dirty source\n").unwrap();
    fs::write(repository.join("untracked.txt"), "untracked source\n").unwrap();

    let result = RepositoryRuntime::default()
        .switch_branch(
            repository,
            &SwitchBranchRequest {
                full_name: "refs/heads/target".to_owned(),
                expected_oid: git_output(repository, &["rev-parse", "refs/heads/target"])
                    .trim()
                    .to_owned(),
                stash_on_dirty: true,
                stash_message: Some("WIP 2026-07-15T15:32:00 main".to_owned()),
            },
        )
        .expect("failed switch after stash is structured");

    assert!(!result.operation_succeeded);
    assert!(!result.changed);
    assert!(result.operation_error.is_some());
    assert_eq!(
        result.auto_stash.restore,
        StashRestoreState::Applied,
        "{:?}",
        result.auto_stash
    );
    assert_eq!(result.auto_stash.cleanup, StashCleanupState::Dropped);
    assert_eq!(
        git_output(repository, &["branch", "--show-current"]).trim(),
        "main"
    );
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "dirty source\n"
    );
    assert!(repository.join("untracked.txt").exists());
    assert!(git_output(repository, &["stash", "list", "--format=%H"]).is_empty());
}

#[test]
fn switches_the_exact_full_ref_when_short_names_are_ambiguous() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "--initial-branch=main"]);
    git(repository, &["config", "user.name", "Switch Test"]);
    git(repository, &["config", "user.email", "switch@example.test"]);
    fs::write(repository.join("tracked.txt"), "base\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    git(repository, &["commit", "-m", "base"]);
    git(repository, &["branch", "topic"]);
    git(repository, &["branch", "refs/heads/topic"]);

    RepositoryRuntime::default()
        .switch_branch(repository, &request(repository, "refs/heads/topic"))
        .expect("switch exact branch");

    let symbolic = Command::new("git")
        .args(["symbolic-ref", "HEAD"])
        .current_dir(repository)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(symbolic.stdout).unwrap().trim(),
        "refs/heads/topic"
    );
}

#[test]
fn rejects_a_real_branch_when_its_oid_no_longer_matches_the_request() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "--initial-branch=main"]);
    git(repository, &["config", "user.name", "Switch Test"]);
    git(repository, &["config", "user.email", "switch@example.test"]);
    fs::write(repository.join("tracked.txt"), "base\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    git(repository, &["commit", "-m", "base"]);
    git(repository, &["branch", "target"]);
    let stale = request(repository, "refs/heads/target");
    fs::write(repository.join("tracked.txt"), "new target\n").unwrap();
    git(repository, &["commit", "-am", "move target source"]);
    git(repository, &["branch", "-f", "target", "HEAD"]);

    let error = RepositoryRuntime::default()
        .switch_branch(repository, &stale)
        .expect_err("stale target must not be switched");

    assert!(matches!(error, BranchSwitchError::TargetChanged));
    assert_eq!(
        git_output(repository, &["branch", "--show-current"]).trim(),
        "main"
    );
}
