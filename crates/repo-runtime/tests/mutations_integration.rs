use std::{fs, path::Path, process::Command};

use app_domain::{
    ApplyIndexChangeRequest, ChangeSelection, CreateCommitRequest, IndexAction, RepositoryStatus,
    StatusEntry, WorkingTreeEntrySelector,
};
use repo_runtime::{MutationRuntimeError, RepositoryRuntime};

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

fn init_repository(with_commit: bool) -> tempfile::TempDir {
    let repository = tempfile::tempdir().expect("temporary repository");
    git(repository.path(), &["init", "-q", "-b", "main"]);
    git(repository.path(), &["config", "user.name", "Mutation Test"]);
    git(
        repository.path(),
        &["config", "user.email", "mutation@example.test"],
    );
    git(repository.path(), &["config", "commit.gpgSign", "false"]);
    if with_commit {
        fs::write(repository.path().join("tracked.txt"), "base\n").unwrap();
        git(repository.path(), &["add", "tracked.txt"]);
        git(repository.path(), &["commit", "-qm", "base"]);
    }
    repository
}

fn selector(entry: &StatusEntry) -> WorkingTreeEntrySelector {
    WorkingTreeEntrySelector {
        path: entry.path.clone(),
        old_path: entry.original_path.clone(),
        entry_kind: entry.kind,
    }
}

fn index_request(
    status: &RepositoryStatus,
    action: IndexAction,
    entries: Vec<WorkingTreeEntrySelector>,
) -> ApplyIndexChangeRequest {
    ApplyIndexChangeRequest {
        action,
        selection: ChangeSelection::Selected { entries },
        expected_head: status.branch.oid.clone(),
        expected_head_name: status.branch.head.clone(),
        expected_detached: status.branch.detached,
        expected_unborn: status.branch.unborn,
        expected_index_fingerprint: status.index_fingerprint.clone(),
        expected_worktree_fingerprint: status.worktree_fingerprint.clone(),
    }
}

#[test]
fn stages_and_unstages_a_partially_staged_file_without_losing_worktree_content() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "staged\n").unwrap();
    git(repository.path(), &["add", "tracked.txt"]);
    fs::write(repository.path().join("tracked.txt"), "latest\n").unwrap();

    let before_stage = runtime.status(repository.path()).unwrap();
    let target = selector(
        before_stage
            .entries
            .iter()
            .find(|entry| entry.path == "tracked.txt")
            .unwrap(),
    );
    let staged = runtime
        .apply_index_change(
            repository.path(),
            &index_request(&before_stage, IndexAction::Stage, vec![target]),
        )
        .unwrap();
    assert!(staged.changed);
    assert!(
        git_output(repository.path(), &["diff", "--quiet"])
            .status
            .success()
    );
    let cached = git_output(repository.path(), &["diff", "--cached"]);
    assert!(String::from_utf8_lossy(&cached.stdout).contains("+latest"));

    let staged_entry = staged
        .status
        .entries
        .iter()
        .find(|entry| entry.path == "tracked.txt")
        .unwrap();
    let unstaged = runtime
        .apply_index_change(
            repository.path(),
            &index_request(
                &staged.status,
                IndexAction::Unstage,
                vec![selector(staged_entry)],
            ),
        )
        .unwrap();
    assert!(unstaged.changed);
    assert!(
        git_output(repository.path(), &["diff", "--cached", "--quiet"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
        "latest\n"
    );
}

#[test]
fn handles_unborn_stage_unstage_and_first_commit() {
    let repository = init_repository(false);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("first.txt"), "first\n").unwrap();

    let untracked = runtime.status(repository.path()).unwrap();
    let first_target = selector(untracked.entries.first().unwrap());
    let staged = runtime
        .apply_index_change(
            repository.path(),
            &index_request(&untracked, IndexAction::Stage, vec![first_target]),
        )
        .unwrap();
    assert!(staged.status.branch.unborn);

    let added_target = selector(staged.status.entries.first().unwrap());
    let unstaged = runtime
        .apply_index_change(
            repository.path(),
            &index_request(&staged.status, IndexAction::Unstage, vec![added_target]),
        )
        .unwrap();
    assert!(repository.path().join("first.txt").is_file());
    assert!(
        unstaged
            .status
            .entries
            .iter()
            .any(|entry| entry.path == "first.txt")
    );

    let restaged = runtime
        .apply_index_change(
            repository.path(),
            &ApplyIndexChangeRequest {
                action: IndexAction::Stage,
                selection: ChangeSelection::All,
                expected_head: unstaged.status.branch.oid.clone(),
                expected_head_name: unstaged.status.branch.head.clone(),
                expected_detached: unstaged.status.branch.detached,
                expected_unborn: unstaged.status.branch.unborn,
                expected_index_fingerprint: unstaged.status.index_fingerprint.clone(),
                expected_worktree_fingerprint: unstaged.status.worktree_fingerprint.clone(),
            },
        )
        .unwrap();
    let committed = runtime
        .create_commit(
            repository.path(),
            &CreateCommitRequest {
                message: "first commit\n\nbody".to_owned(),
                expected_head: restaged.status.branch.oid.clone(),
                expected_head_name: restaged.status.branch.head.clone(),
                expected_detached: restaged.status.branch.detached,
                expected_unborn: restaged.status.branch.unborn,
                expected_index_fingerprint: restaged.status.index_fingerprint.clone(),
                expected_worktree_fingerprint: restaged.status.worktree_fingerprint.clone(),
            },
        )
        .unwrap();
    assert_eq!(committed.oid.len(), 40);
    assert!(!committed.status.branch.unborn);
    assert!(committed.status.entries.is_empty());
}

#[test]
fn rejects_a_stale_index_without_mutating_it() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "changed\n").unwrap();
    let stale = runtime.status(repository.path()).unwrap();
    let target = selector(stale.entries.first().unwrap());

    fs::write(repository.path().join("external.txt"), "external\n").unwrap();
    git(repository.path(), &["add", "external.txt"]);
    let index_before = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;
    let result = runtime.apply_index_change(
        repository.path(),
        &index_request(&stale, IndexAction::Stage, vec![target]),
    );
    let index_after = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;

    assert!(matches!(result, Err(MutationRuntimeError::StaleState)));
    assert_eq!(index_before, index_after);
}

#[test]
fn rejects_stage_when_the_selected_worktree_file_changed_after_snapshot() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "first change\n").unwrap();
    let stale = runtime.status(repository.path()).unwrap();
    let target = selector(stale.entries.first().unwrap());

    fs::write(
        repository.path().join("tracked.txt"),
        "a different and longer second change\n",
    )
    .unwrap();
    let result = runtime.apply_index_change(
        repository.path(),
        &index_request(&stale, IndexAction::Stage, vec![target]),
    );

    assert!(matches!(result, Err(MutationRuntimeError::StaleState)));
    assert!(
        git_output(repository.path(), &["diff", "--cached", "--quiet"])
            .status
            .success()
    );
}

#[test]
fn stages_only_the_selected_literal_unicode_path() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    let selected_name = "--dosłowny [plik]*.txt";
    let other_name = "other.txt";
    fs::write(repository.path().join(selected_name), "selected\n").unwrap();
    fs::write(repository.path().join(other_name), "other\n").unwrap();

    let before = runtime.status(repository.path()).unwrap();
    let selected = before
        .entries
        .iter()
        .find(|entry| entry.path == selected_name)
        .map(selector)
        .unwrap();
    let result = runtime
        .apply_index_change(
            repository.path(),
            &index_request(&before, IndexAction::Stage, vec![selected]),
        )
        .unwrap();

    assert!(result.changed);
    let staged = git_output(
        repository.path(),
        &["diff", "--cached", "--name-only", "-z"],
    );
    assert_eq!(staged.stdout, format!("{selected_name}\0").into_bytes());
    assert!(repository.path().join(other_name).is_file());
    assert!(result.status.entries.iter().any(|entry| {
        entry.path == other_name && entry.kind == app_domain::StatusEntryKind::Untracked
    }));
}

#[test]
fn unstages_a_selected_rename_without_losing_either_path() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    let old_name = "tracked.txt";
    let new_name = "renamed.txt";
    git(repository.path(), &["mv", old_name, new_name]);

    let before = runtime.status(repository.path()).unwrap();
    let rename = before
        .entries
        .iter()
        .find(|entry| entry.kind == app_domain::StatusEntryKind::RenamedOrCopied)
        .map(selector)
        .expect("staged rename");
    let result = runtime
        .apply_index_change(
            repository.path(),
            &index_request(&before, IndexAction::Unstage, vec![rename]),
        )
        .unwrap();

    assert!(result.changed);
    assert!(
        git_output(repository.path(), &["diff", "--cached", "--quiet"])
            .status
            .success()
    );
    assert!(!repository.path().join(old_name).exists());
    assert_eq!(
        fs::read_to_string(repository.path().join(new_name)).unwrap(),
        "base\n"
    );
}

#[test]
fn rejects_a_commit_when_the_index_fingerprint_is_stale() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "staged\n").unwrap();
    git(repository.path(), &["add", "tracked.txt"]);
    let stale = runtime.status(repository.path()).unwrap();
    let head_before = stale.branch.oid.clone();

    fs::write(repository.path().join("external.txt"), "external\n").unwrap();
    git(repository.path(), &["add", "external.txt"]);
    let index_before = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;
    let result = runtime.create_commit(
        repository.path(),
        &CreateCommitRequest {
            message: "must not commit stale state".to_owned(),
            expected_head: stale.branch.oid,
            expected_head_name: stale.branch.head,
            expected_detached: stale.branch.detached,
            expected_unborn: stale.branch.unborn,
            expected_index_fingerprint: stale.index_fingerprint,
            expected_worktree_fingerprint: stale.worktree_fingerprint,
        },
    );

    assert!(matches!(result, Err(MutationRuntimeError::StaleState)));
    assert_eq!(
        runtime.status(repository.path()).unwrap().branch.oid,
        head_before
    );
    assert_eq!(
        git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout,
        index_before
    );
}

#[test]
fn rejects_a_commit_after_switching_to_another_branch_at_the_same_oid() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "staged\n").unwrap();
    git(repository.path(), &["add", "tracked.txt"]);
    git(repository.path(), &["branch", "other"]);
    let stale = runtime.status(repository.path()).unwrap();
    let head_before = stale.branch.oid.clone();

    git(repository.path(), &["switch", "-q", "other"]);
    let result = runtime.create_commit(
        repository.path(),
        &CreateCommitRequest {
            message: "must remain on the intended branch".to_owned(),
            expected_head: stale.branch.oid,
            expected_head_name: stale.branch.head,
            expected_detached: stale.branch.detached,
            expected_unborn: stale.branch.unborn,
            expected_index_fingerprint: stale.index_fingerprint,
            expected_worktree_fingerprint: stale.worktree_fingerprint,
        },
    );

    assert!(matches!(result, Err(MutationRuntimeError::StaleState)));
    let after = runtime.status(repository.path()).unwrap();
    assert_eq!(after.branch.head.as_deref(), Some("other"));
    assert_eq!(after.branch.oid, head_before);
}

#[cfg(unix)]
#[test]
fn commit_runs_hooks_and_preserves_head_and_index_on_failure() {
    use std::os::unix::fs::PermissionsExt;

    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "staged\n").unwrap();
    git(repository.path(), &["add", "tracked.txt"]);
    let before = runtime.status(repository.path()).unwrap();
    let head_before = before.branch.oid.clone();
    let index_before = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;
    let hook = repository.path().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho hook-blocked >&2\nexit 23\n").unwrap();
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).unwrap();

    let result = runtime.create_commit(
        repository.path(),
        &CreateCommitRequest {
            message: "must fail".to_owned(),
            expected_head: before.branch.oid.clone(),
            expected_head_name: before.branch.head.clone(),
            expected_detached: before.branch.detached,
            expected_unborn: before.branch.unborn,
            expected_index_fingerprint: before.index_fingerprint.clone(),
            expected_worktree_fingerprint: before.worktree_fingerprint.clone(),
        },
    );

    assert!(result.unwrap_err().to_string().contains("hook-blocked"));
    assert_eq!(
        runtime.status(repository.path()).unwrap().branch.oid,
        head_before
    );
    assert_eq!(
        git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout,
        index_before
    );
}

#[cfg(unix)]
#[test]
fn commit_keeps_signing_enabled_and_preserves_state_when_signing_fails() {
    use std::os::unix::fs::PermissionsExt;

    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "signed\n").unwrap();
    git(repository.path(), &["add", "tracked.txt"]);
    let signer = repository.path().join("reject-signing.sh");
    fs::write(&signer, "#!/bin/sh\necho signing-blocked >&2\nexit 29\n").unwrap();
    let mut permissions = fs::metadata(&signer).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&signer, permissions).unwrap();
    git(
        repository.path(),
        &["config", "gpg.program", signer.to_str().unwrap()],
    );
    git(repository.path(), &["config", "commit.gpgSign", "true"]);
    let before = runtime.status(repository.path()).unwrap();
    let head_before = before.branch.oid.clone();
    let index_before = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;

    let result = runtime.create_commit(
        repository.path(),
        &CreateCommitRequest {
            message: "must fail signing".to_owned(),
            expected_head: before.branch.oid,
            expected_head_name: before.branch.head,
            expected_detached: before.branch.detached,
            expected_unborn: before.branch.unborn,
            expected_index_fingerprint: before.index_fingerprint,
            expected_worktree_fingerprint: before.worktree_fingerprint,
        },
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("signing-blocked") || error.contains("failed to write commit object"));
    assert_eq!(
        runtime.status(repository.path()).unwrap().branch.oid,
        head_before
    );
    assert_eq!(
        git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout,
        index_before
    );
}
