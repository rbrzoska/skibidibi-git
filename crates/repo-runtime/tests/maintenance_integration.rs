use std::{fs, path::Path, process::Command};

use app_domain::{DeleteBranchRequest, RemoveWorktreeRequest, WorktreeRemovalMode};
use repo_runtime::{MaintenanceError, RepositoryRuntime};

fn git_output(repository: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .expect("run git")
}

fn git(repository: &Path, arguments: &[&str]) {
    let output = git_output(repository, arguments);
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn output_text(repository: &Path, arguments: &[&str]) -> String {
    String::from_utf8(git_output(repository, arguments).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "-q", "-b", "main"]);
    git(
        directory.path(),
        &["config", "user.name", "Maintenance Test"],
    );
    git(
        directory.path(),
        &["config", "user.email", "maintenance@example.test"],
    );
    fs::write(directory.path().join("tracked.txt"), "base\n").unwrap();
    git(directory.path(), &["add", "tracked.txt"]);
    git(directory.path(), &["commit", "-qm", "base"]);
    directory
}

#[test]
fn deletes_an_exact_merged_local_branch_but_not_a_checked_out_branch() {
    let repository = repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["branch", "merged"]);
    let oid = String::from_utf8(git_output(repository.path(), &["rev-parse", "merged"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    let result = runtime
        .delete_branch(
            repository.path(),
            &DeleteBranchRequest {
                full_name: "refs/heads/merged".to_owned(),
                expected_oid: oid,
            },
        )
        .unwrap();
    assert!(result.deleted);
    assert!(
        !git_output(
            repository.path(),
            &["show-ref", "--verify", "refs/heads/merged"]
        )
        .status
        .success()
    );

    let head = String::from_utf8(git_output(repository.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    let error = runtime
        .delete_branch(
            repository.path(),
            &DeleteBranchRequest {
                full_name: "refs/heads/main".to_owned(),
                expected_oid: head,
            },
        )
        .unwrap_err();
    assert!(matches!(error, MaintenanceError::CurrentBranch));
}

#[test]
fn deletes_the_exact_full_ref_when_short_names_are_ambiguous() {
    let repository = repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["branch", "topic"]);
    git(repository.path(), &["branch", "refs/heads/topic"]);
    let oid = String::from_utf8(git_output(repository.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();

    runtime
        .delete_branch(
            repository.path(),
            &DeleteBranchRequest {
                full_name: "refs/heads/topic".to_owned(),
                expected_oid: oid,
            },
        )
        .unwrap();

    assert!(
        !git_output(
            repository.path(),
            &["show-ref", "--verify", "refs/heads/topic"]
        )
        .status
        .success()
    );
    assert!(
        git_output(
            repository.path(),
            &["show-ref", "--verify", "refs/heads/refs/heads/topic"]
        )
        .status
        .success()
    );
}

#[test]
fn removes_a_clean_linked_worktree_and_its_associated_branch() {
    let repository = repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["branch", "feature/worktree"]);
    let linked_root = tempfile::tempdir().unwrap();
    let linked = linked_root.path().join("linked tree");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "feature/worktree",
        ],
    );
    let navigation = runtime.navigation(repository.path()).unwrap();
    let worktree = navigation
        .worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some("refs/heads/feature/worktree"))
        .unwrap();

    let result = runtime
        .remove_worktree_and_branch(
            repository.path(),
            &RemoveWorktreeRequest {
                path: worktree.path.clone(),
                expected_head: worktree.head.clone(),
                branch_full_name: worktree.branch.clone(),
                mode: WorktreeRemovalMode::Safe,
                stash_message: None,
            },
        )
        .unwrap();

    assert!(result.branch_deleted);
    assert!(result.worktree_removed);
    assert_eq!(result.branch_deletion_error, None);
    assert!(!linked.exists());
    assert!(
        !git_output(
            repository.path(),
            &["show-ref", "--verify", "refs/heads/feature/worktree"]
        )
        .status
        .success()
    );
}

#[test]
fn refuses_to_delete_an_unmerged_local_branch() {
    let repository = repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["switch", "-c", "unmerged"]);
    fs::write(repository.path().join("branch-only.txt"), "branch-only\n").unwrap();
    git(repository.path(), &["add", "branch-only.txt"]);
    git(repository.path(), &["commit", "-qm", "branch-only"]);
    let oid = String::from_utf8(git_output(repository.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned();
    git(repository.path(), &["switch", "main"]);

    let error = runtime
        .delete_branch(
            repository.path(),
            &DeleteBranchRequest {
                full_name: "refs/heads/unmerged".to_owned(),
                expected_oid: oid,
            },
        )
        .unwrap_err();

    assert!(matches!(error, MaintenanceError::Git(_)));
    assert!(
        git_output(
            repository.path(),
            &["show-ref", "--verify", "refs/heads/unmerged"]
        )
        .status
        .success()
    );
}

#[test]
fn refuses_to_remove_a_dirty_worktree_without_force_and_keeps_its_branch() {
    let repository = repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["branch", "dirty-worktree"]);
    let linked_root = tempfile::tempdir().unwrap();
    let linked = linked_root.path().join("dirty linked");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "dirty-worktree",
        ],
    );
    fs::write(linked.join("untracked.txt"), "preserve\n").unwrap();
    let worktree = runtime
        .navigation(repository.path())
        .unwrap()
        .worktrees
        .into_iter()
        .find(|worktree| worktree.branch.as_deref() == Some("refs/heads/dirty-worktree"))
        .unwrap();

    let result = runtime
        .remove_worktree_and_branch(
            repository.path(),
            &RemoveWorktreeRequest {
                path: worktree.path,
                expected_head: worktree.head,
                branch_full_name: worktree.branch,
                mode: WorktreeRemovalMode::Safe,
                stash_message: None,
            },
        )
        .unwrap();

    assert!(!result.worktree_removed);
    assert!(result.worktree_removal_error.is_some());
    assert_eq!(
        fs::read_to_string(linked.join("untracked.txt")).unwrap(),
        "preserve\n"
    );
    assert!(
        git_output(
            repository.path(),
            &["show-ref", "--verify", "refs/heads/dirty-worktree"]
        )
        .status
        .success()
    );
}

#[test]
fn force_removes_a_dirty_worktree_and_its_branch_without_creating_a_stash() {
    let repository = repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["branch", "force-worktree"]);
    let linked_root = tempfile::tempdir().unwrap();
    let linked = linked_root.path().join("force linked");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "force-worktree",
        ],
    );
    fs::write(linked.join("tracked.txt"), "discarded\n").unwrap();
    fs::write(linked.join("untracked.txt"), "discarded\n").unwrap();
    let worktree = runtime
        .navigation(repository.path())
        .unwrap()
        .worktrees
        .into_iter()
        .find(|worktree| worktree.branch.as_deref() == Some("refs/heads/force-worktree"))
        .unwrap();

    let result = runtime
        .remove_worktree_and_branch(
            repository.path(),
            &RemoveWorktreeRequest {
                path: worktree.path,
                expected_head: worktree.head,
                branch_full_name: worktree.branch,
                mode: WorktreeRemovalMode::Force,
                stash_message: None,
            },
        )
        .unwrap();

    assert!(result.worktree_removed);
    assert!(result.branch_deleted);
    assert!(result.stash.is_none());
    assert!(!linked.exists());
    assert!(output_text(repository.path(), &["stash", "list"]).is_empty());
}

#[test]
fn stash_and_force_preserves_staged_unstaged_and_untracked_changes_before_removal() {
    let repository = repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["branch", "stash-force-worktree"]);
    let linked_root = tempfile::tempdir().unwrap();
    let linked = linked_root.path().join("stash force linked");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "stash-force-worktree",
        ],
    );
    fs::write(linked.join("tracked.txt"), "staged\n").unwrap();
    git(&linked, &["add", "tracked.txt"]);
    fs::write(linked.join("tracked.txt"), "worktree\n").unwrap();
    fs::write(linked.join("untracked.txt"), "untracked\n").unwrap();
    let worktree = runtime
        .navigation(repository.path())
        .unwrap()
        .worktrees
        .into_iter()
        .find(|worktree| worktree.branch.as_deref() == Some("refs/heads/stash-force-worktree"))
        .unwrap();
    let message = "WIP 2026-07-16T05:00:00 stash-force-worktree";

    let result = runtime
        .remove_worktree_and_branch(
            repository.path(),
            &RemoveWorktreeRequest {
                path: worktree.path,
                expected_head: worktree.head,
                branch_full_name: worktree.branch,
                mode: WorktreeRemovalMode::StashAndForce,
                stash_message: Some(message.to_owned()),
            },
        )
        .unwrap();

    assert!(result.worktree_removed);
    assert!(result.branch_deleted);
    assert!(result.stash.is_some());
    assert!(output_text(repository.path(), &["stash", "list", "--format=%gs"]).contains(message));
    assert!(!linked.exists());

    git(
        repository.path(),
        &["stash", "apply", "--index", "stash@{0}"],
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
        "worktree\n"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("untracked.txt")).unwrap(),
        "untracked\n"
    );
    assert!(
        output_text(
            repository.path(),
            &["diff", "--cached", "--", "tracked.txt"]
        )
        .contains("+staged")
    );
}

#[test]
fn fetches_remote_updates_without_changing_head_or_fetch_head() {
    let repository = repository();
    let remote_root = tempfile::tempdir().unwrap();
    let remote = remote_root.path().join("remote.git");
    let init = Command::new("git")
        .args(["init", "--bare", "-q", remote.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(init.status.success());
    git(
        repository.path(),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(repository.path(), &["push", "-u", "origin", "main"]);

    let producer_root = tempfile::tempdir().unwrap();
    let producer = producer_root.path().join("producer");
    let clone = Command::new("git")
        .args([
            "clone",
            "-q",
            "--branch",
            "main",
            remote.to_str().unwrap(),
            producer.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(clone.status.success());
    git(&producer, &["config", "user.name", "Producer"]);
    git(
        &producer,
        &["config", "user.email", "producer@example.test"],
    );
    fs::write(producer.join("remote.txt"), "remote update\n").unwrap();
    git(&producer, &["add", "remote.txt"]);
    git(&producer, &["commit", "-qm", "remote update"]);
    git(&producer, &["push", "-q", "origin", "main"]);

    let head_before = git_output(repository.path(), &["rev-parse", "HEAD"]).stdout;
    let result = RepositoryRuntime::default()
        .fetch_repository(repository.path())
        .unwrap();

    assert!(result.fetched_at > 0);
    assert_eq!(
        git_output(repository.path(), &["rev-parse", "HEAD"]).stdout,
        head_before
    );
    assert_eq!(
        git_output(repository.path(), &["rev-parse", "origin/main"]).stdout,
        git_output(&producer, &["rev-parse", "HEAD"]).stdout
    );
    assert!(!repository.path().join(".git/FETCH_HEAD").exists());
}
