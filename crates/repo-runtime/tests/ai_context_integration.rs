use std::{fs, path::Path, process::Command};

use repo_runtime::staged_ai_context_default;

fn git(repository: &Path, arguments: &[&str]) {
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
}

#[test]
fn context_contains_only_the_staged_snapshot_from_an_isolated_repository() {
    let temporary = tempfile::tempdir().unwrap();
    git(temporary.path(), &["init", "-q"]);
    git(temporary.path(), &["config", "user.name", "Fixture"]);
    git(
        temporary.path(),
        &["config", "user.email", "fixture@example.test"],
    );
    fs::write(temporary.path().join("message.txt"), "initial\n").unwrap();
    git(temporary.path(), &["add", "message.txt"]);
    git(temporary.path(), &["commit", "-qm", "Initial"]);

    fs::write(temporary.path().join("message.txt"), "staged marker\n").unwrap();
    git(temporary.path(), &["add", "message.txt"]);
    fs::write(
        temporary.path().join("message.txt"),
        "unstaged private marker\n",
    )
    .unwrap();

    let context = staged_ai_context_default(temporary.path()).unwrap();
    assert!(context.text.contains("staged marker"));
    assert!(!context.text.contains("unstaged private marker"));
}
