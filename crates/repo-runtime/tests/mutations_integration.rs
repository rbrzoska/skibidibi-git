use std::{fs, path::Path, process::Command};

use app_domain::{
    AmendCommitRequest, AmendCommitState, ApplyIndexChangeRequest, ChangeSelection,
    CreateCommitRequest, IndexAction, RepositoryStatus, StatusEntry, WorkingTreeEntrySelector,
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

fn amend_request(
    status: &RepositoryStatus,
    message: Option<&str>,
    confirm_upstream_rewrite: bool,
) -> AmendCommitRequest {
    AmendCommitRequest {
        message: message.map(str::to_owned),
        confirm_upstream_rewrite,
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
fn amend_with_a_new_message_commits_staged_content_and_preserves_the_parent_set() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    let parents_before =
        git_output(repository.path(), &["show", "-s", "--format=%P", "HEAD"]).stdout;
    fs::write(repository.path().join("tracked.txt"), "amended\n").unwrap();
    git(repository.path(), &["add", "tracked.txt"]);
    let before = runtime.status(repository.path()).unwrap();
    let result = runtime
        .amend_commit(
            repository.path(),
            &amend_request(&before, Some("replacement subject\n\nbody"), false),
        )
        .unwrap();

    assert_eq!(result.previous_oid, before.branch.oid.unwrap());
    assert_eq!(result.state, AmendCommitState::Succeeded);
    assert_eq!(
        result.oid,
        result
            .status
            .as_ref()
            .and_then(|status| status.branch.oid.clone())
    );
    assert_eq!(
        git_output(repository.path(), &["show", "-s", "--format=%P", "HEAD"]).stdout,
        parents_before
    );
    assert_eq!(
        String::from_utf8(
            git_output(repository.path(), &["show", "-s", "--format=%B", "HEAD"]).stdout
        )
        .unwrap(),
        "replacement subject\n\nbody\n\n"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join("tracked.txt")).unwrap(),
        "amended\n"
    );
    assert!(result.status.as_ref().unwrap().entries.is_empty());
}

#[test]
fn amend_without_editing_the_message_succeeds_without_staged_changes() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    let message_before =
        git_output(repository.path(), &["show", "-s", "--format=%B", "HEAD"]).stdout;
    let tree_before = git_output(repository.path(), &["show", "-s", "--format=%T", "HEAD"]).stdout;
    let parents_before =
        git_output(repository.path(), &["show", "-s", "--format=%P", "HEAD"]).stdout;
    let before = runtime.status(repository.path()).unwrap();

    let result = runtime
        .amend_commit(repository.path(), &amend_request(&before, None, false))
        .unwrap();

    assert_eq!(result.previous_oid, before.branch.oid.unwrap());
    assert_eq!(
        git_output(repository.path(), &["show", "-s", "--format=%B", "HEAD"]).stdout,
        message_before
    );
    assert_eq!(
        git_output(repository.path(), &["show", "-s", "--format=%T", "HEAD"]).stdout,
        tree_before
    );
    assert_eq!(
        git_output(repository.path(), &["show", "-s", "--format=%P", "HEAD"]).stdout,
        parents_before
    );
}

#[test]
fn amend_requires_confirmation_when_head_is_on_the_configured_upstream() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    git(
        repository.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repository.git",
        ],
    );
    git(
        repository.path(),
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    git(
        repository.path(),
        &["branch", "--set-upstream-to=origin/main", "main"],
    );
    let before = runtime.status(repository.path()).unwrap();
    assert_eq!(before.branch.upstream.as_deref(), Some("origin/main"));
    assert_eq!(before.branch.ahead, 0);
    let head_before = before.branch.oid.clone();

    let rejected = runtime.amend_commit(
        repository.path(),
        &amend_request(&before, Some("published replacement"), false),
    );
    assert!(matches!(
        rejected,
        Err(MutationRuntimeError::UpstreamRewriteConfirmationRequired)
    ));
    assert_eq!(
        runtime.status(repository.path()).unwrap().branch.oid,
        head_before
    );

    let confirmed = runtime
        .amend_commit(
            repository.path(),
            &amend_request(&before, Some("published replacement"), true),
        )
        .unwrap();
    assert_eq!(
        String::from_utf8(
            git_output(repository.path(), &["show", "-s", "--format=%s", "HEAD"]).stdout
        )
        .unwrap()
        .trim(),
        "published replacement"
    );
    assert_ne!(
        confirmed.oid.as_deref(),
        Some(confirmed.previous_oid.as_str())
    );
}

#[test]
fn amend_of_an_unpushed_head_does_not_require_upstream_confirmation() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    git(
        repository.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repository.git",
        ],
    );
    git(
        repository.path(),
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    git(
        repository.path(),
        &["branch", "--set-upstream-to=origin/main", "main"],
    );
    fs::write(repository.path().join("local.txt"), "local\n").unwrap();
    git(repository.path(), &["add", "local.txt"]);
    git(repository.path(), &["commit", "-qm", "local"]);
    let before = runtime.status(repository.path()).unwrap();
    assert_eq!(before.branch.ahead, 1);

    runtime
        .amend_commit(
            repository.path(),
            &amend_request(&before, Some("local replacement"), false),
        )
        .unwrap();
}

#[test]
fn amend_rejects_unborn_head_and_invalid_messages_without_mutation() {
    let repository = init_repository(false);
    let runtime = RepositoryRuntime::default();
    let before = runtime.status(repository.path()).unwrap();
    let no_head = runtime.amend_commit(repository.path(), &amend_request(&before, None, false));
    assert!(matches!(no_head, Err(MutationRuntimeError::NothingToAmend)));

    let invalid = runtime.amend_commit(
        repository.path(),
        &amend_request(&before, Some(" \n\t"), false),
    );
    assert!(matches!(
        invalid,
        Err(MutationRuntimeError::InvalidCommitMessage { .. })
    ));
    assert!(runtime.status(repository.path()).unwrap().branch.unborn);
}

#[test]
fn amend_rejects_a_stale_index_without_mutating_head_or_index() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    let stale = runtime.status(repository.path()).unwrap();
    let head_before = stale.branch.oid.clone();
    fs::write(repository.path().join("external.txt"), "external\n").unwrap();
    git(repository.path(), &["add", "external.txt"]);
    let index_before = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;

    let result = runtime.amend_commit(
        repository.path(),
        &amend_request(&stale, Some("must not amend stale state"), false),
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
fn amend_rejects_conflicted_entries_without_mutating_head_or_index() {
    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["switch", "-qc", "other"]);
    fs::write(repository.path().join("tracked.txt"), "other\n").unwrap();
    git(repository.path(), &["commit", "-qam", "other"]);
    git(repository.path(), &["switch", "-q", "main"]);
    fs::write(repository.path().join("tracked.txt"), "main\n").unwrap();
    git(repository.path(), &["commit", "-qam", "main"]);
    let merge = git_output(repository.path(), &["merge", "other"]);
    assert!(!merge.status.success());
    let before = runtime.status(repository.path()).unwrap();
    let head_before = before.branch.oid.clone();
    let index_before = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;

    let result = runtime.amend_commit(
        repository.path(),
        &amend_request(&before, Some("must not amend conflicts"), false),
    );

    assert!(matches!(
        result,
        Err(MutationRuntimeError::ConflictsPresent)
    ));
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
fn amend_runs_hooks_and_preserves_head_and_index_when_a_hook_rejects_it() {
    use std::os::unix::fs::PermissionsExt;

    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    fs::write(repository.path().join("tracked.txt"), "staged amend\n").unwrap();
    git(repository.path(), &["add", "tracked.txt"]);
    let hook = repository.path().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho amend-hook-blocked >&2\nexit 31\n").unwrap();
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).unwrap();
    let before = runtime.status(repository.path()).unwrap();
    let head_before = before.branch.oid.clone();
    let index_before = git_output(repository.path(), &["ls-files", "--stage", "-z"]).stdout;

    let result = runtime.amend_commit(
        repository.path(),
        &amend_request(&before, Some("hooked amend"), false),
    );

    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("amend-hook-blocked")
    );
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
fn amend_reports_an_unknown_outcome_when_a_hook_moves_head_during_commit() {
    use std::os::unix::fs::PermissionsExt;

    let repository = init_repository(true);
    let runtime = RepositoryRuntime::default();
    git(repository.path(), &["switch", "-qc", "external"]);
    fs::write(repository.path().join("external.txt"), "external\n").unwrap();
    git(repository.path(), &["add", "external.txt"]);
    git(repository.path(), &["commit", "-qm", "external"]);
    let external_oid =
        String::from_utf8(git_output(repository.path(), &["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_owned();
    git(repository.path(), &["switch", "-q", "main"]);

    let hook = repository.path().join(".git/hooks/pre-commit");
    fs::write(
        &hook,
        format!("#!/bin/sh\ngit update-ref refs/heads/main {external_oid}\n"),
    )
    .unwrap();
    let mut permissions = fs::metadata(&hook).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook, permissions).unwrap();
    let before = runtime.status(repository.path()).unwrap();

    let result = runtime
        .amend_commit(repository.path(), &amend_request(&before, None, false))
        .expect("post-mutation uncertainty is returned as a structured outcome");

    assert_eq!(result.state, AmendCommitState::OutcomeUnknown);
    assert!(result.error_message.is_some());
    assert_eq!(
        result
            .status
            .as_ref()
            .and_then(|status| status.branch.oid.as_deref()),
        Some(external_oid.as_str())
    );
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
