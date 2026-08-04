use std::{fs, path::Path, process::Command};

use app_domain::{
    CommitOperationRequest, CommitOperationState, RepositoryStatePrecondition, RepositoryStatus,
    ResetCommitRequest, ResetMode, StatusCode, StatusEntryKind,
};
use repo_runtime::{CommitOperationError, RepositoryRuntime};

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

fn git_stdout(repository: &Path, arguments: &[&str]) -> String {
    let output = git_output(repository, arguments);
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git output is UTF-8")
        .trim()
        .to_owned()
}

fn init_repository() -> tempfile::TempDir {
    let repository = tempfile::tempdir().expect("temporary repository");
    git(repository.path(), &["init", "-q", "-b", "main"]);
    git(repository.path(), &["config", "core.autocrlf", "false"]);
    git(
        repository.path(),
        &["config", "user.name", "Commit Operation Test"],
    );
    git(
        repository.path(),
        &["config", "user.email", "commit-operation@example.test"],
    );
    git(repository.path(), &["config", "commit.gpgSign", "false"]);
    fs::write(repository.path().join("tracked.txt"), "base\n").expect("write base");
    git(repository.path(), &["add", "tracked.txt"]);
    git(repository.path(), &["commit", "-qm", "base"]);
    repository
}

fn commit_file(repository: &Path, content: &str, message: &str) -> String {
    fs::write(repository.join("tracked.txt"), content).expect("write tracked file");
    git(repository, &["add", "tracked.txt"]);
    git(repository, &["commit", "-qm", message]);
    git_stdout(repository, &["rev-parse", "HEAD"])
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

fn commit_request(status: &RepositoryStatus, target_oid: &str) -> CommitOperationRequest {
    CommitOperationRequest {
        target_oid: target_oid.to_owned(),
        precondition: precondition(status),
    }
}

fn reset_request(
    status: &RepositoryStatus,
    target_oid: &str,
    mode: ResetMode,
    confirm_hard_reset: bool,
) -> ResetCommitRequest {
    ResetCommitRequest {
        target_oid: target_oid.to_owned(),
        mode,
        confirm_hard_reset,
        precondition: precondition(status),
    }
}

fn divergent_commit(repository: &Path, content: &str) -> String {
    git(repository, &["switch", "-qc", "source"]);
    let target = commit_file(repository, content, "source change");
    git(repository, &["switch", "-q", "main"]);
    target
}

#[test]
fn cherry_picks_and_reverts_one_exact_non_merge_commit() {
    let repository = init_repository();
    let runtime = RepositoryRuntime::default();
    let target = divergent_commit(repository.path(), "source\n");

    let before = runtime
        .status(repository.path())
        .expect("status before pick");
    let picked = runtime
        .cherry_pick_commit(repository.path(), &commit_request(&before, &target))
        .expect("cherry-pick result");
    assert_eq!(picked.state, CommitOperationState::Succeeded);
    assert_eq!(picked.target_oid, target);
    assert_eq!(picked.head_before, before.branch.oid.unwrap());
    assert_ne!(
        picked.head_after.as_deref(),
        Some(picked.head_before.as_str())
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
        "source\n"
    );
    assert!(
        picked
            .status
            .as_ref()
            .expect("fresh status")
            .entries
            .is_empty()
    );

    let before_revert = runtime
        .status(repository.path())
        .expect("status before revert");
    let reverted = runtime
        .revert_commit(repository.path(), &commit_request(&before_revert, &target))
        .expect("revert result");
    assert_eq!(reverted.state, CommitOperationState::Succeeded);
    assert_eq!(
        fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
        "base\n"
    );
    assert!(
        reverted
            .status
            .as_ref()
            .expect("fresh status")
            .entries
            .is_empty()
    );
}

#[test]
fn returns_a_fresh_conflicted_status_for_a_cherry_pick_conflict() {
    let repository = init_repository();
    let runtime = RepositoryRuntime::default();
    let target = divergent_commit(repository.path(), "source\n");
    commit_file(repository.path(), "main\n", "main change");
    let before = runtime
        .status(repository.path())
        .expect("status before pick");

    let result = runtime
        .cherry_pick_commit(repository.path(), &commit_request(&before, &target))
        .expect("structured conflict");

    assert_eq!(result.state, CommitOperationState::Conflicted);
    assert_eq!(result.head_after, Some(result.head_before.clone()));
    assert!(result.error_message.is_some());
    assert!(
        result
            .status
            .expect("conflict status")
            .entries
            .iter()
            .any(|entry| entry.kind == StatusEntryKind::Unmerged)
    );
    assert!(result.mutation_may_have_occurred);
}

#[test]
fn rejects_dirty_stale_and_merge_commit_requests_before_mutation() {
    let repository = init_repository();
    let runtime = RepositoryRuntime::default();
    let target = divergent_commit(repository.path(), "source\n");
    let clean = runtime.status(repository.path()).expect("clean status");
    fs::write(
        repository.path().join("untracked.txt"),
        "dirty and different\n",
    )
    .unwrap();
    let dirty = runtime.status(repository.path()).expect("dirty status");

    let stale_error = runtime
        .cherry_pick_commit(repository.path(), &commit_request(&clean, &target))
        .expect_err("stale snapshot must fail");
    assert!(matches!(stale_error, CommitOperationError::StaleState));
    let dirty_error = runtime
        .cherry_pick_commit(repository.path(), &commit_request(&dirty, &target))
        .expect_err("dirty tree must fail");
    assert!(matches!(
        dirty_error,
        CommitOperationError::DirtyWorkingTree
    ));
    fs::remove_file(repository.path().join("untracked.txt")).unwrap();

    git(repository.path(), &["switch", "-qc", "merge-source"]);
    fs::write(repository.path().join("merge-only.txt"), "feature\n").unwrap();
    git(repository.path(), &["add", "merge-only.txt"]);
    git(repository.path(), &["commit", "-qm", "feature"]);
    git(repository.path(), &["switch", "-q", "main"]);
    fs::write(repository.path().join("main-only.txt"), "main\n").unwrap();
    git(repository.path(), &["add", "main-only.txt"]);
    git(repository.path(), &["commit", "-qm", "main"]);
    git(
        repository.path(),
        &["merge", "--no-ff", "-qm", "merge commit", "merge-source"],
    );
    let merge_oid = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let before_merge_request = runtime.status(repository.path()).unwrap();
    let head_before = before_merge_request.branch.oid.clone();

    let merge_error = runtime
        .revert_commit(
            repository.path(),
            &commit_request(&before_merge_request, &merge_oid),
        )
        .expect_err("merge revert must be rejected");
    assert!(matches!(
        merge_error,
        CommitOperationError::MergeCommitUnsupported
    ));
    assert_eq!(
        runtime.status(repository.path()).unwrap().branch.oid,
        head_before
    );
}

#[test]
fn empty_cherry_pick_is_an_outcome_unknown_not_a_safe_retry() {
    let repository = init_repository();
    let runtime = RepositoryRuntime::default();
    let target = divergent_commit(repository.path(), "source\n");
    let before = runtime.status(repository.path()).unwrap();
    runtime
        .cherry_pick_commit(repository.path(), &commit_request(&before, &target))
        .unwrap();
    let already_applied = runtime.status(repository.path()).unwrap();

    let result = runtime
        .cherry_pick_commit(
            repository.path(),
            &commit_request(&already_applied, &target),
        )
        .expect("empty pick returns a conservative result");

    assert_eq!(result.state, CommitOperationState::OutcomeUnknown);
    assert!(result.status.is_some());
    assert!(result.error_message.is_some());
    assert!(result.mutation_may_have_occurred);
}

#[test]
fn reset_supports_soft_mixed_and_confirmed_hard_modes() {
    for mode in [ResetMode::Soft, ResetMode::Mixed, ResetMode::Hard] {
        let repository = init_repository();
        let runtime = RepositoryRuntime::default();
        let base = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
        commit_file(repository.path(), "later\n", "later");
        let before = runtime.status(repository.path()).unwrap();

        if mode == ResetMode::Hard {
            let error = runtime
                .reset_commit(
                    repository.path(),
                    &reset_request(&before, &base, mode, false),
                )
                .expect_err("hard reset requires explicit confirmation");
            assert!(matches!(
                error,
                CommitOperationError::HardResetConfirmationRequired
            ));
            assert_eq!(
                runtime.status(repository.path()).unwrap().branch.oid,
                before.branch.oid
            );
        }

        let result = runtime
            .reset_commit(
                repository.path(),
                &reset_request(&before, &base, mode, mode == ResetMode::Hard),
            )
            .expect("reset result");
        assert_eq!(result.state, CommitOperationState::Succeeded);
        assert_eq!(result.head_after.as_deref(), Some(base.as_str()));
        let status = result.status.expect("fresh reset status");
        match mode {
            ResetMode::Soft => assert!(
                status
                    .entries
                    .iter()
                    .any(|entry| entry.index_status != StatusCode::Unmodified)
            ),
            ResetMode::Mixed => assert!(
                status
                    .entries
                    .iter()
                    .any(|entry| entry.worktree_status != StatusCode::Unmodified)
            ),
            ResetMode::Hard => {
                assert!(status.entries.is_empty());
                assert_eq!(
                    fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
                    "base\n"
                );
            }
        }
    }
}

#[test]
fn hard_reset_accepts_an_exact_merge_commit_target() {
    let repository = init_repository();
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["switch", "-qc", "feature"]);
    fs::write(repository.path().join("feature.txt"), "feature\n").unwrap();
    git(repository.path(), &["add", "feature.txt"]);
    git(repository.path(), &["commit", "-qm", "feature"]);
    git(repository.path(), &["switch", "-q", "main"]);
    fs::write(repository.path().join("main.txt"), "main\n").unwrap();
    git(repository.path(), &["add", "main.txt"]);
    git(repository.path(), &["commit", "-qm", "main"]);
    git(
        repository.path(),
        &["merge", "--no-ff", "-qm", "merge", "feature"],
    );
    let merge_oid = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    commit_file(repository.path(), "after merge\n", "after merge");
    let before = runtime.status(repository.path()).unwrap();

    let result = runtime
        .reset_commit(
            repository.path(),
            &reset_request(&before, &merge_oid, ResetMode::Hard, true),
        )
        .expect("reset to merge commit");

    assert_eq!(result.state, CommitOperationState::Succeeded);
    assert_eq!(result.head_after, Some(merge_oid));
}

#[test]
fn rejects_non_exact_or_injection_shaped_object_ids() {
    let repository = init_repository();
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(repository.path()).unwrap();
    let head_before = before.branch.oid.clone();
    for target in ["HEAD", "-a", "a;touch-pwned", "a/b", &"a".repeat(39)] {
        let error = runtime
            .reset_commit(
                repository.path(),
                &reset_request(&before, target, ResetMode::Mixed, false),
            )
            .expect_err("non-exact oid must fail");
        assert!(matches!(error, CommitOperationError::InvalidRequest));
    }
    assert_eq!(
        runtime.status(repository.path()).unwrap().branch.oid,
        head_before
    );
    assert!(!repository.path().join("touch-pwned").exists());
}
