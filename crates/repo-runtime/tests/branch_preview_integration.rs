use std::{fs, path::Path, process::Command};

use app_domain::CommitRelation;
use repo_runtime::RepositoryRuntime;
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) -> String {
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
    String::from_utf8(output.stdout)
        .expect("git output")
        .trim()
        .to_owned()
}

fn commit_file(repository: &Path, name: &str, contents: &str, message: &str) -> String {
    fs::write(repository.join(name), contents).expect("write fixture file");
    git(repository, &["add", "--", name]);
    git(repository, &["commit", "-m", message]);
    git(repository, &["rev-parse", "HEAD"])
}

fn commit_tree(repository: &Path, parent: &str, message: &str) -> String {
    let tree = git(repository, &["rev-parse", &format!("{parent}^{{tree}}")]);
    git(
        repository,
        &["commit-tree", &tree, "-p", parent, "-m", message],
    )
}

fn commit_merge(
    repository: &Path,
    first_parent: &str,
    second_parent: &str,
    message: &str,
) -> String {
    let tree = git(
        repository,
        &["rev-parse", &format!("{first_parent}^{{tree}}")],
    );
    git(
        repository,
        &[
            "commit-tree",
            &tree,
            "-p",
            first_parent,
            "-p",
            second_parent,
            "-m",
            message,
        ],
    )
}

#[test]
fn previews_an_exact_branch_and_classifies_task_merge_and_base_commits() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "Preview Test"]);
    git(
        repository,
        &["config", "user.email", "preview@example.test"],
    );

    commit_file(repository, "base.txt", "base\n", "base");
    git(repository, &["branch", "release"]);
    git(repository, &["switch", "release"]);
    let release_oid = commit_file(repository, "release.txt", "release\n", "release change");

    git(repository, &["switch", "main"]);
    git(repository, &["switch", "-c", "feature/task"]);
    commit_file(repository, "task.txt", "task one\n", "task one");
    git(repository, &["switch", "-c", "feature/side"]);
    commit_file(repository, "side.txt", "side\n", "side change");
    git(repository, &["switch", "feature/task"]);
    git(
        repository,
        &["merge", "--no-ff", "feature/side", "-m", "merge side"],
    );
    git(
        repository,
        &["merge", "--no-ff", "release", "-m", "merge release"],
    );
    let feature_oid = commit_file(repository, "task.txt", "task two\n", "task two");

    let runtime = RepositoryRuntime::default();
    let page = runtime
        .branch_history_page(
            repository,
            "refs/heads/feature/task",
            &feature_oid,
            "refs/heads/release",
            &release_oid,
            20,
            None,
        )
        .expect("branch preview");

    let relation = |summary: &str| {
        page.commits
            .iter()
            .find(|commit| commit.summary == summary)
            .and_then(|commit| commit.relation)
    };
    assert_eq!(relation("task two"), Some(CommitRelation::Task));
    assert_eq!(relation("merge release"), Some(CommitRelation::Merge));
    assert_eq!(relation("merge side"), Some(CommitRelation::Merge));
    assert_eq!(relation("side change"), Some(CommitRelation::Merge));
    assert_eq!(relation("task one"), Some(CommitRelation::Task));
    assert_eq!(relation("release change"), Some(CommitRelation::Base));
    assert_eq!(relation("base"), Some(CommitRelation::Base));
}

#[test]
fn paginates_deep_history_without_losing_or_misclassifying_commits() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "Preview Test"]);
    git(
        repository,
        &["config", "user.email", "preview@example.test"],
    );
    let base_oid = commit_file(repository, "base.txt", "base\n", "base");

    let mut target_oid = base_oid.clone();
    for index in 0..18 {
        target_oid = commit_tree(repository, &target_oid, &format!("target {index}"));
    }
    git(
        repository,
        &["update-ref", "refs/heads/release", &target_oid],
    );

    let mut feature_oid = base_oid;
    for index in 0..32 {
        feature_oid = commit_tree(repository, &feature_oid, &format!("task before {index}"));
    }
    let side_base = feature_oid.clone();
    let mut side_oid = side_base;
    for index in 0..9 {
        side_oid = commit_tree(repository, &side_oid, &format!("side {index}"));
    }
    feature_oid = commit_merge(repository, &feature_oid, &side_oid, "merge side history");
    feature_oid = commit_merge(
        repository,
        &feature_oid,
        &target_oid,
        "merge target history",
    );
    for index in 0..37 {
        feature_oid = commit_tree(repository, &feature_oid, &format!("task after {index}"));
    }
    git(
        repository,
        &["update-ref", "refs/heads/feature/deep", &feature_oid],
    );

    let runtime = RepositoryRuntime::default();
    let mut cursor = None;
    let mut commits = Vec::new();
    loop {
        let page = runtime
            .branch_history_page(
                repository,
                "refs/heads/feature/deep",
                &feature_oid,
                "refs/heads/release",
                &target_oid,
                11,
                cursor.as_deref(),
            )
            .expect("paginated branch preview");
        commits.extend(page.commits);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    assert!(commits.len() > 90, "fixture should span many pages");
    for commit in commits {
        let expected = if commit.summary == "base" || commit.summary.starts_with("target ") {
            CommitRelation::Base
        } else if commit.summary.starts_with("side ") || commit.summary.starts_with("merge ") {
            CommitRelation::Merge
        } else {
            CommitRelation::Task
        };
        assert_eq!(
            commit.relation,
            Some(expected),
            "unexpected relation for {}",
            commit.summary
        );
    }
}

#[test]
fn rejects_a_branch_that_moved_after_the_ui_snapshot() {
    let directory = tempdir().expect("temporary repository");
    let repository = directory.path();
    git(repository, &["init", "-b", "main"]);
    git(repository, &["config", "user.name", "Preview Test"]);
    git(
        repository,
        &["config", "user.email", "preview@example.test"],
    );
    let oid = commit_file(repository, "base.txt", "base\n", "base");
    git(repository, &["branch", "release"]);
    git(repository, &["branch", "feature"]);

    let runtime = RepositoryRuntime::default();
    let error = runtime
        .branch_history_page(
            repository,
            "refs/heads/feature",
            &"f".repeat(40),
            "refs/heads/release",
            &oid,
            20,
            None,
        )
        .expect_err("stale branch oid must be rejected");

    assert!(error.to_string().contains("branch changed"));
}
