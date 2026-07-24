use std::{fs, path::Path, process::Command};

use repo_runtime::task_review_context_default;

fn git(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("Git fixture command should start");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn review_context_combines_my_branch_commits_and_all_uncommitted_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = temporary.path();
    git(repository, &["init", "-q", "-b", "main"]);
    git(repository, &["config", "user.name", "Review Author"]);
    git(
        repository,
        &["config", "user.email", "reviewer@example.test"],
    );
    fs::write(repository.join("shared.txt"), "initial\n").unwrap();
    git(repository, &["add", "shared.txt"]);
    git(repository, &["commit", "-qm", "Initial"]);

    git(repository, &["checkout", "-qb", "feature/task"]);
    fs::write(repository.join("task.txt"), "committed task marker\n").unwrap();
    git(repository, &["add", "task.txt"]);
    git(repository, &["commit", "-qm", "Implement task behavior"]);

    git(repository, &["checkout", "-q", "main"]);
    fs::write(repository.join("release.txt"), "release marker\n").unwrap();
    git(repository, &["add", "release.txt"]);
    git(repository, &["commit", "-qm", "Advance release"]);
    let target = git(repository, &["rev-parse", "HEAD"]);
    git(repository, &["checkout", "-q", "feature/task"]);

    fs::write(
        repository.join("task.txt"),
        "committed task marker\nstaged marker\n",
    )
    .unwrap();
    git(repository, &["add", "task.txt"]);
    fs::write(repository.join("shared.txt"), "unstaged marker\n").unwrap();
    fs::write(repository.join("new-file.txt"), "untracked marker\n").unwrap();
    fs::write(repository.join(".env.local"), "SECRET=must-not-leak\n").unwrap();

    let context = task_review_context_default(repository, &target).unwrap();
    assert_eq!(context.branch, "feature/task");
    assert!(!context.target_merged);
    assert_eq!(context.my_commits.len(), 1);
    assert_eq!(context.my_commits[0].summary, "Implement task behavior");
    assert!(context.changed_files.contains(&"task.txt".to_owned()));
    assert!(context.changed_files.contains(&"shared.txt".to_owned()));
    assert!(context.changed_files.contains(&"new-file.txt".to_owned()));
    assert!(context.text.contains("committed task marker"));
    assert!(context.text.contains("staged marker"));
    assert!(context.text.contains("unstaged marker"));
    assert!(context.text.contains("untracked marker"));
    assert!(!context.text.contains("must-not-leak"));
    assert!(context.text.contains("Sensitive paths omitted"));
}
