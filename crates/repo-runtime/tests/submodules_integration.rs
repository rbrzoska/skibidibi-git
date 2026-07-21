use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use app_domain::{SubmoduleCommitState, SubmoduleWorktreeState};
use repo_runtime::RepositoryRuntime;
use tempfile::tempdir;

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .expect("git is installed");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repository: &Path, arguments: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()
        .expect("git is installed");
    assert!(output.status.success(), "git {arguments:?} failed");
    output.stdout
}

fn git_with_input(repository: &Path, arguments: &[&str], input: &[u8]) {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git is installed");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input)
        .expect("write git stdin");
    let output = child.wait_with_output().expect("wait for git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn large_index_without_gitlinks_returns_an_empty_submodule_list() {
    let temporary = tempdir().expect("temporary directory");
    let repository = temporary.path().join("large-parent");
    fs::create_dir_all(&repository).expect("repository directory");
    git(&repository, &["init", "-q"]);
    let blob = String::from_utf8(git_output(&repository, &["hash-object", "-w", "--stdin"]))
        .expect("blob oid")
        .trim()
        .to_owned();
    let mut index = Vec::new();
    for number in 0..12_000 {
        index.extend_from_slice(
            format!(
                "100644 {blob}\tgenerated/very-long-directory-name-for-large-index-regression/file-{number:05}.txt\0"
            )
            .as_bytes(),
        );
    }
    git_with_input(&repository, &["update-index", "-z", "--index-info"], &index);
    assert!(git_output(&repository, &["ls-files", "--stage", "-z"]).len() > 1024 * 1024);

    let result = RepositoryRuntime::default()
        .submodules(&repository)
        .expect("large repository without submodules");

    assert!(result.submodules.is_empty());
}

#[test]
fn lists_an_initialized_immediate_submodule_and_its_child_changes() {
    let temporary = tempdir().expect("temporary directory");
    let source = temporary.path().join("source");
    let parent = temporary.path().join("parent");
    fs::create_dir_all(&source).expect("source directory");
    fs::create_dir_all(&parent).expect("parent directory");

    git(&source, &["init", "-q"]);
    git(&source, &["config", "user.email", "test@example.com"]);
    git(&source, &["config", "user.name", "Test User"]);
    fs::write(source.join("tracked.txt"), "before\n").expect("source file");
    git(&source, &["add", "tracked.txt"]);
    git(&source, &["commit", "-qm", "source initial"]);

    git(&parent, &["init", "-q"]);
    git(&parent, &["config", "user.email", "test@example.com"]);
    git(&parent, &["config", "user.name", "Test User"]);
    git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "../source",
            "modules/client",
        ],
    );
    git(&parent, &["commit", "-qam", "add client"]);

    let runtime = RepositoryRuntime::default();
    let clean = runtime.submodules(&parent).expect("clean submodules");
    assert_eq!(clean.submodules.len(), 1);
    let client = &clean.submodules[0];
    assert_eq!(client.name, "modules/client");
    assert_eq!(client.path, "modules/client");
    assert!(client.present && client.initialized);
    assert_eq!(client.commit_state, SubmoduleCommitState::AtExpected);
    assert_eq!(client.worktree_state, SubmoduleWorktreeState::Clean);
    assert_eq!(client.change_count, 0);
    assert_eq!(client.current_oid, client.expected_oid);

    let child = parent.join("modules/client");
    fs::write(child.join("tracked.txt"), "after\n").expect("modified child file");
    fs::write(child.join("untracked.txt"), "untracked\n").expect("untracked child file");
    let dirty = runtime.submodules(&parent).expect("dirty submodules");
    let client = &dirty.submodules[0];
    assert_eq!(
        client.worktree_state,
        SubmoduleWorktreeState::ModifiedAndUntracked
    );
    assert_eq!(client.change_count, 2);
    assert_eq!(client.commit_state, SubmoduleCommitState::AtExpected);
}
