use std::{fs, path::Path, process::Command};

use app_domain::SwitchBranchRequest;
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

fn request(full_name: &str) -> SwitchBranchRequest {
    SwitchBranchRequest {
        full_name: full_name.to_owned(),
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
        .switch_branch(repository, &request("refs/heads/main"))
        .expect_err("Git must reject overwriting the dirty file");
    assert!(matches!(error, BranchSwitchError::DirtyWorktree));
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).expect("read preserved dirty file"),
        "dirty feature version\n"
    );

    git(repository, &["restore", "--", "tracked.txt"]);
    let result = runtime
        .switch_branch(repository, &request("refs/heads/main"))
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
                stash_on_dirty: true,
                stash_message: Some("WIP 2026-07-15T15:30:00 feature".to_owned()),
            },
        )
        .expect("stash and switch");

    assert!(result.changed);
    assert!(result.stash_created);
    assert_eq!(
        String::from_utf8(
            Command::new("git")
                .args(["stash", "list", "--format=%gs"])
                .current_dir(repository)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim(),
        "On feature: WIP 2026-07-15T15:30:00 feature"
    );
    assert!(!repository.join("untracked.txt").exists());
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "base\n"
    );
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
        .switch_branch(repository, &request("refs/heads/topic"))
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
