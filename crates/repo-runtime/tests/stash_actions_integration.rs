use std::{fs, path::Path, process::Command};

use app_domain::{
    ApplyStashRequest, DropStashRequest, PopStashRequest, PushStashRequest,
    RepositoryStatePrecondition, RepositoryStatus, StashCleanupState, StashPushState,
    StashRestoreState,
};
use repo_runtime::{RepositoryRuntime, StashActionError};
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should be installed");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Git test output is UTF-8")
}

fn repository() -> tempfile::TempDir {
    let directory = tempdir().expect("temporary repository");
    let path = directory.path();
    git(path, &["init", "--initial-branch=main"]);
    git(path, &["config", "core.autocrlf", "false"]);
    git(path, &["config", "user.name", "Stash Test"]);
    git(path, &["config", "user.email", "stash@example.test"]);
    fs::write(path.join("tracked.txt"), "base\n").expect("write base file");
    git(path, &["add", "--", "tracked.txt"]);
    git(path, &["commit", "-m", "base"]);
    directory
}

fn precondition(status: &RepositoryStatus) -> RepositoryStatePrecondition {
    RepositoryStatePrecondition {
        expected_head: status.branch.oid.clone(),
        expected_head_name: status.branch.head.clone(),
        expected_detached: status.branch.detached,
        expected_unborn: status.branch.unborn,
        expected_index_fingerprint: status.index_fingerprint.clone(),
        expected_worktree_fingerprint: status.worktree_fingerprint.clone(),
    }
}

fn push(
    runtime: &RepositoryRuntime,
    repository: &Path,
    message: &str,
) -> app_domain::StashIdentity {
    let status = runtime.status(repository).expect("status before stash");
    let result = runtime
        .push_stash(
            repository,
            &PushStashRequest {
                message: message.to_owned(),
                include_untracked: true,
                precondition: precondition(&status),
            },
        )
        .expect("push stash");
    assert_eq!(result.state, StashPushState::Created);
    result.stash.expect("created stash identity")
}

#[test]
fn push_and_apply_restore_index_worktree_and_untracked_files() {
    let directory = repository();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();

    fs::write(repository.join("tracked.txt"), "staged\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    fs::write(repository.join("tracked.txt"), "unstaged\n").unwrap();
    fs::write(repository.join("untracked.txt"), "untracked\n").unwrap();
    let stash = push(&runtime, repository, "manual snapshot");

    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "base\n"
    );
    assert!(!repository.join("untracked.txt").exists());
    let clean = runtime.status(repository).unwrap();
    let applied = runtime
        .apply_stash(
            repository,
            &ApplyStashRequest {
                stash: stash.clone(),
                restore_index: true,
                precondition: precondition(&clean),
            },
        )
        .expect("apply stash");

    assert_eq!(applied.restore, StashRestoreState::Applied);
    assert_eq!(applied.cleanup, StashCleanupState::Retained);
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(repository.join("untracked.txt")).unwrap(),
        "untracked\n"
    );
    assert!(git(repository, &["diff", "--cached", "--", "tracked.txt"]).contains("+staged"));
    assert!(git(repository, &["diff", "--", "tracked.txt"]).contains("+unstaged"));
    assert_eq!(
        git(repository, &["stash", "list", "--format=%H"]).trim(),
        stash.oid
    );
    assert!(
        git(
            repository,
            &["for-each-ref", "--format=%(refname)", "refs/skibidibi"]
        )
        .is_empty()
    );
}

#[test]
fn push_without_untracked_reports_no_changes_when_only_untracked_files_exist() {
    let directory = repository();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    fs::write(repository.join("untracked.txt"), "leave me\n").unwrap();
    let status = runtime.status(repository).unwrap();

    let result = runtime
        .push_stash(
            repository,
            &PushStashRequest {
                message: "tracked files only".to_owned(),
                include_untracked: false,
                precondition: precondition(&status),
            },
        )
        .expect("untracked-only state is a no-op");

    assert_eq!(result.state, StashPushState::NoChanges);
    assert!(result.stash.is_none());
    assert!(repository.join("untracked.txt").exists());
    assert!(git(repository, &["stash", "list", "--format=%H"]).is_empty());
}

#[test]
fn pop_applies_then_drops_the_exact_stash() {
    let directory = repository();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    fs::write(repository.join("tracked.txt"), "saved\n").unwrap();
    let stash = push(&runtime, repository, "pop snapshot");
    let clean = runtime.status(repository).unwrap();

    let popped = runtime
        .pop_stash(
            repository,
            &PopStashRequest {
                stash,
                restore_index: true,
                precondition: precondition(&clean),
            },
        )
        .expect("pop stash");

    assert_eq!(popped.restore, StashRestoreState::Applied);
    assert_eq!(popped.cleanup, StashCleanupState::Dropped);
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "saved\n"
    );
    assert!(git(repository, &["stash", "list", "--format=%H"]).is_empty());
}

#[test]
fn conflicted_pop_retains_the_stash() {
    let directory = repository();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    fs::write(repository.join("tracked.txt"), "stashed side\n").unwrap();
    let stash = push(&runtime, repository, "conflicting snapshot");
    fs::write(repository.join("tracked.txt"), "committed side\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    git(repository, &["commit", "-m", "conflicting commit"]);
    let clean = runtime.status(repository).unwrap();

    let popped = runtime
        .pop_stash(
            repository,
            &PopStashRequest {
                stash: stash.clone(),
                restore_index: true,
                precondition: precondition(&clean),
            },
        )
        .expect("conflict is a structured pop outcome");

    assert_eq!(popped.restore, StashRestoreState::Conflicted);
    assert_eq!(popped.cleanup, StashCleanupState::Retained);
    assert!(
        popped
            .status
            .as_ref()
            .expect("post-conflict status")
            .entries
            .iter()
            .any(|entry| entry.kind == app_domain::StatusEntryKind::Unmerged)
    );
    assert_eq!(
        git(repository, &["stash", "list", "--format=%H"]).trim(),
        stash.oid
    );
}

#[test]
fn tracked_only_push_uses_exact_created_oid_and_cleans_the_worktree() {
    let directory = repository();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    fs::write(repository.join("tracked.txt"), "staged snapshot\n").unwrap();
    git(repository, &["add", "--", "tracked.txt"]);
    fs::write(repository.join("tracked.txt"), "unstaged snapshot\n").unwrap();
    let status = runtime.status(repository).unwrap();

    let result = runtime
        .push_stash(
            repository,
            &PushStashRequest {
                message: "exact tracked snapshot".to_owned(),
                include_untracked: false,
                precondition: precondition(&status),
            },
        )
        .expect("exact tracked stash");

    assert_eq!(result.state, StashPushState::Created);
    let stash = result.stash.expect("stored identity");
    assert_eq!(result.mutation_oid.as_deref(), Some(stash.oid.as_str()));
    assert_eq!(
        fs::read_to_string(repository.join("tracked.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(
        git(repository, &["stash", "list", "--format=%H"]).trim(),
        stash.oid
    );
    assert!(
        git(
            repository,
            &["for-each-ref", "--format=%(refname)", "refs/skibidibi"]
        )
        .is_empty(),
        "successful exact creation must release its safety pin"
    );

    let clean = runtime.status(repository).unwrap();
    let applied = runtime
        .apply_stash(
            repository,
            &ApplyStashRequest {
                stash,
                restore_index: true,
                precondition: precondition(&clean),
            },
        )
        .expect("apply exact tracked stash");
    assert_eq!(applied.restore, StashRestoreState::Applied);
    assert!(
        git(repository, &["diff", "--cached", "--", "tracked.txt"]).contains("+staged snapshot")
    );
    assert!(git(repository, &["diff", "--", "tracked.txt"]).contains("+unstaged snapshot"));
}

#[test]
fn drop_rebinds_an_older_stash_by_oid_without_deleting_the_newer_one() {
    let directory = repository();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    fs::write(repository.join("tracked.txt"), "first\n").unwrap();
    let first = push(&runtime, repository, "first snapshot");
    fs::write(repository.join("tracked.txt"), "second\n").unwrap();
    let second = push(&runtime, repository, "second snapshot");

    let dropped = runtime
        .drop_stash(repository, &DropStashRequest { stash: first })
        .expect("drop older stash");

    assert_eq!(dropped.cleanup, StashCleanupState::Dropped);
    assert_eq!(
        git(repository, &["stash", "list", "--format=%H"]).trim(),
        second.oid
    );
}

#[test]
fn drop_refuses_an_oid_stored_in_the_reflog_more_than_once() {
    let directory = repository();
    let repository = directory.path();
    let runtime = RepositoryRuntime::default();
    fs::write(repository.join("tracked.txt"), "duplicate me\n").unwrap();
    let stash = push(&runtime, repository, "original snapshot");
    fs::write(repository.join("tracked.txt"), "different snapshot\n").unwrap();
    let _different = push(&runtime, repository, "different snapshot");
    git(
        repository,
        &[
            "stash",
            "store",
            "--message",
            "duplicate snapshot",
            &stash.oid,
        ],
    );

    let error = runtime
        .drop_stash(repository, &DropStashRequest { stash })
        .expect_err("ambiguous OID must never select a reflog entry");

    assert!(matches!(error, StashActionError::AmbiguousStash));
    assert_eq!(
        git(repository, &["stash", "list", "--format=%H"])
            .lines()
            .count(),
        3
    );
}
