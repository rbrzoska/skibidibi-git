use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use thiserror::Error;

const DEFAULT_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitInvocationPolicy {
    /// Commands that only inspect repository state. Git lock creation is disabled.
    ReadOnly,
    /// Commands that intentionally modify repository or working-tree state.
    Mutating,
    /// Commands that contact a remote. Authentication must be supplied externally.
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInvocation {
    policy: GitInvocationPolicy,
    arguments: Vec<OsString>,
    stdin: Option<Vec<u8>>,
    stdout_limit: usize,
    stderr_limit: usize,
    timeout: Option<Duration>,
}

impl GitInvocation {
    pub fn new<I, S>(policy: GitInvocationPolicy, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            policy,
            arguments: arguments.into_iter().map(Into::into).collect(),
            stdin: None,
            stdout_limit: DEFAULT_OUTPUT_LIMIT,
            stderr_limit: DEFAULT_OUTPUT_LIMIT,
            timeout: None,
        }
    }

    pub fn with_stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub fn with_output_limits(mut self, stdout: usize, stderr: usize) -> Self {
        self.stdout_limit = stdout;
        self.stderr_limit = stderr;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn policy(&self) -> GitInvocationPolicy {
        self.policy
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOutputStream {
    Stdout,
    Stderr,
}

impl std::fmt::Display for GitOutputStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdout => formatter.write_str("stdout"),
            Self::Stderr => formatter.write_str("stderr"),
        }
    }
}

#[derive(Debug, Error)]
pub enum GitRunError {
    #[error("failed to start git: {0}")]
    Spawn(#[source] io::Error),
    #[error("failed while communicating with git: {0}")]
    Io(#[source] io::Error),
    #[error("git timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
    #[error("git {stream} exceeded the configured {limit}-byte limit")]
    OutputLimitExceeded {
        stream: GitOutputStream,
        limit: usize,
    },
    #[error("command {command:?} is not permitted by the read-only Git API")]
    ReadOnlyPolicyViolation { command: String },
    #[error("git exited unsuccessfully ({code:?}): {stderr}")]
    Unsuccessful { code: Option<i32>, stderr: String },
}

pub trait GitExecutor: Send + Sync {
    /// Executes one of the runner's allowlisted read-only Git commands.
    fn execute(&self, repository: &Path, arguments: &[&str]) -> Result<GitOutput, GitRunError>;
}

/// Executes Git directly with an argv vector. No command is ever passed through a shell.
///
/// A future cancellation token can be added alongside the timeout in `run`; process waiting is
/// deliberately centralized there. Cancellation is not coupled to an async runtime in this crate.
/// Git starts in an isolated process group where the platform supports it so timeout and output
/// limits can terminate helpers which inherited Git's output pipes.
#[derive(Debug, Clone)]
pub struct GitRunner {
    executable: PathBuf,
}

impl Default for GitRunner {
    fn default() -> Self {
        Self::new("git")
    }
}

impl GitRunner {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn run(
        &self,
        repository: &Path,
        invocation: GitInvocation,
    ) -> Result<GitOutput, GitRunError> {
        if invocation.policy == GitInvocationPolicy::ReadOnly {
            validate_read_only_command(&invocation.arguments)?;
        }

        let mut command = Command::new(&self.executable);
        command
            .arg("--no-pager")
            .arg("-c")
            .arg("core.fsmonitor=false")
            .arg("-C")
            .arg(repository)
            .args(&invocation.arguments)
            .env("LC_ALL", "C")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        match invocation.policy {
            GitInvocationPolicy::ReadOnly => {
                command.env("GIT_OPTIONAL_LOCKS", "0");
            }
            GitInvocationPolicy::Mutating | GitInvocationPolicy::Network => {
                // Do not let a parent process configured for read-only Git disable locks here.
                command.env_remove("GIT_OPTIONAL_LOCKS");
            }
        }
        if invocation.policy == GitInvocationPolicy::Network {
            command
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GCM_INTERACTIVE", "Never");
        }

        if invocation.stdin.is_some() {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }

        configure_process_tree(&mut command);

        let mut child = command.spawn().map_err(GitRunError::Spawn)?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let stdout_limit = invocation.stdout_limit;
        let stderr_limit = invocation.stderr_limit;
        let (limit_sender, limit_receiver) = mpsc::channel();
        let stdout_sender = limit_sender.clone();
        let stdout_reader = thread::spawn(move || {
            read_bounded(stdout, stdout_limit, GitOutputStream::Stdout, stdout_sender)
        });
        let stderr_reader = thread::spawn(move || {
            read_bounded(stderr, stderr_limit, GitOutputStream::Stderr, limit_sender)
        });

        let stdin_writer = invocation.stdin.map(|input| {
            let mut stdin = child.stdin.take().expect("stdin is piped");
            thread::spawn(move || stdin.write_all(&input))
        });

        let completion = wait_for_exit(
            &mut child,
            invocation.timeout,
            &limit_receiver,
            invocation.stdout_limit,
            invocation.stderr_limit,
        );
        if let Err(error) = completion {
            // Kill descendants before reaping Git. A platform fallback may still fail, so do not
            // synchronously join pipe readers on this error path: inherited helper handles must
            // never defeat the caller's timeout.
            terminate_process_tree(&mut child);
            child.wait().map_err(GitRunError::Io)?;
            return Err(error);
        }

        let stdin_result = stdin_writer.map(join_io_thread).transpose();
        let stdout_result = join_io_thread(stdout_reader);
        let stderr_result = join_io_thread(stderr_reader);

        let status = completion.expect("checked successful completion");
        stdin_result?;
        let stdout = stdout_result?;
        let stderr = stderr_result?;

        if stdout.exceeded {
            return Err(GitRunError::OutputLimitExceeded {
                stream: GitOutputStream::Stdout,
                limit: invocation.stdout_limit,
            });
        }
        if stderr.exceeded {
            return Err(GitRunError::OutputLimitExceeded {
                stream: GitOutputStream::Stderr,
                limit: invocation.stderr_limit,
            });
        }
        if !status.success() {
            return Err(GitRunError::Unsuccessful {
                code: status.code(),
                stderr: String::from_utf8_lossy(&stderr.bytes).trim().to_owned(),
            });
        }

        Ok(GitOutput {
            stdout: stdout.bytes,
            stderr: stderr.bytes,
        })
    }

    /// Compatibility entry point for allowlisted read-only commands.
    ///
    /// Mutating and network operations must use [`Self::run`] with an explicit policy.
    pub fn execute_os<I, S>(
        &self,
        repository: &Path,
        arguments: I,
    ) -> Result<GitOutput, GitRunError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let arguments = arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_os_string());
        self.run(
            repository,
            GitInvocation::new(GitInvocationPolicy::ReadOnly, arguments),
        )
    }
}

#[cfg(unix)]
fn configure_process_tree(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn configure_process_tree(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn configure_process_tree(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_process_tree(child: &mut std::process::Child) {
    let process_group = -(child.id() as i32);
    // SAFETY: the child was spawned as leader of a fresh process group. SIGKILL is used only after
    // the bounded invocation has already timed out or exceeded its output limit.
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
    let _ = child.kill();
}

#[cfg(windows)]
fn terminate_process_tree(child: &mut std::process::Child) {
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let _ = Command::new(system_root.join("System32").join("taskkill.exe"))
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

impl GitExecutor for GitRunner {
    fn execute(&self, repository: &Path, arguments: &[&str]) -> Result<GitOutput, GitRunError> {
        self.execute_os(repository, arguments)
    }
}

fn validate_read_only_command(arguments: &[OsString]) -> Result<(), GitRunError> {
    let values = arguments
        .iter()
        .map(|argument| argument.to_str())
        .collect::<Option<Vec<_>>>();
    let is_read_only = values.as_deref().is_some_and(is_allowed_read_only_shape);
    if is_read_only {
        return Ok(());
    }

    Err(GitRunError::ReadOnlyPolicyViolation {
        command: arguments
            .first()
            .map(|command| command.to_string_lossy().into_owned())
            .unwrap_or_else(|| "<missing>".to_owned()),
    })
}

fn is_allowed_read_only_shape(arguments: &[&str]) -> bool {
    match arguments {
        ["status"] => true,
        [
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ] => true,
        ["rev-parse", "--is-inside-work-tree"] => true,
        ["rev-parse", "--verify", "--quiet", "HEAD"] => true,
        ["symbolic-ref", "--quiet", "--short", "HEAD"] => true,
        ["config", "--get", key] => safe_config_key(key),
        ["remote"] => true,
        ["remote", "get-url", "--", remote] => safe_remote_name(remote),
        ["remote", "get-url", "--push", "--", remote] => safe_remote_name(remote),
        ["worktree", "list", "--porcelain", "-z"] => true,
        ["for-each-ref", rest @ ..] => allowed_for_each_ref(rest),
        ["log", rest @ ..] => allowed_log(rest),
        ["show", rest @ ..] => allowed_show(rest),
        ["diff", rest @ ..] => allowed_working_tree_diff(rest),
        ["hash-object", "-t", "tree", "--stdin"] => true,
        ["ls-files", "--stage", "-z"] => true,
        ["cat-file", "blob", oid] => valid_object_id(oid),
        ["merge-base", "--is-ancestor", oid, "HEAD"] => valid_object_id(oid),
        _ => false,
    }
}

fn safe_config_key(key: &str) -> bool {
    let Some(branch) = key
        .strip_prefix("branch.")
        .and_then(|key| key.strip_suffix(".remote"))
    else {
        return false;
    };
    !branch.is_empty() && !branch.chars().any(char::is_control)
}

fn safe_remote_name(remote: &str) -> bool {
    !remote.is_empty() && !remote.chars().any(char::is_control)
}

fn allowed_for_each_ref(arguments: &[&str]) -> bool {
    !arguments.is_empty()
        && arguments.iter().all(|argument| {
            argument.starts_with("--format=")
                || argument.strip_prefix("--count=").is_some_and(ascii_digits)
                || allowed_ref_filter(argument)
        })
        && arguments
            .iter()
            .any(|argument| argument.starts_with("--format="))
        && arguments
            .iter()
            .any(|argument| allowed_ref_filter(argument))
}

fn allowed_ref_filter(value: &str) -> bool {
    matches!(value, "refs/heads" | "refs/remotes" | "refs/stash")
        || value.strip_prefix("refs/heads/").is_some_and(|branch| {
            !branch.is_empty()
                && !branch.starts_with('-')
                && !branch.chars().any(char::is_control)
                && !branch.contains("..")
                && !branch.contains("@{")
        })
}

fn allowed_log(arguments: &[&str]) -> bool {
    !arguments.is_empty()
        && arguments.iter().all(|argument| {
            matches!(
                *argument,
                "-z" | "-g" | "--topo-order" | "--date-order" | "refs/stash"
            ) || argument
                .strip_prefix("--max-count=")
                .is_some_and(ascii_digits)
                || argument.strip_prefix("--skip=").is_some_and(ascii_digits)
                || argument.starts_with("--format=")
                || valid_object_id(argument)
        })
        && arguments
            .iter()
            .any(|argument| argument.starts_with("--format="))
        && arguments
            .iter()
            .any(|argument| *argument == "refs/stash" || valid_object_id(argument))
}

fn allowed_show(arguments: &[&str]) -> bool {
    if let [
        "--format=",
        "--first-parent",
        "--no-ext-diff",
        "--no-textconv",
        "--name-status" | "--numstat",
        "-z",
        "-M",
        oid,
        "--",
    ] = arguments
    {
        return valid_object_id(oid);
    }

    if let [
        "--format=",
        "--first-parent",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "-M",
        "--unified=2147483647",
        oid,
        "--",
        pathspecs @ ..,
    ] = arguments
    {
        return valid_object_id(oid)
            && matches!(pathspecs.len(), 1 | 2)
            && pathspecs
                .iter()
                .all(|pathspec| valid_literal_pathspec(pathspec));
    }

    if let [
        "--format=",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "-M",
        "--unified=2147483647",
        oid,
        "--",
        pathspecs @ ..,
    ] = arguments
    {
        return valid_object_id(oid)
            && matches!(pathspecs.len(), 1 | 2)
            && pathspecs
                .iter()
                .all(|pathspec| valid_literal_pathspec(pathspec));
    }

    arguments.last() == Some(&"--")
        && arguments.iter().all(|argument| {
            matches!(
                *argument,
                "-s" | "-z"
                    | "--format="
                    | "--first-parent"
                    | "--name-status"
                    | "--numstat"
                    | "-M"
                    | "--"
            ) || argument.starts_with("--format=")
                || valid_object_id(argument)
        })
        && arguments.iter().any(|argument| valid_object_id(argument))
        && arguments
            .iter()
            .any(|argument| matches!(*argument, "-s" | "--name-status" | "--numstat"))
}

fn allowed_working_tree_diff(arguments: &[&str]) -> bool {
    let arguments = arguments.strip_prefix(&["--cached"]).unwrap_or(arguments);
    let [
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "-M",
        "--unified=2147483647",
        base,
        "--",
        pathspecs @ ..,
    ] = arguments
    else {
        return false;
    };
    (*base == "HEAD" || valid_object_id(base))
        && matches!(pathspecs.len(), 1 | 2)
        && pathspecs
            .iter()
            .all(|pathspec| valid_literal_pathspec(pathspec))
}

fn valid_literal_pathspec(value: &str) -> bool {
    value
        .strip_prefix(":(literal)")
        .is_some_and(|path| !path.is_empty() && !path.contains('\0'))
}

fn ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct BoundedOutput {
    bytes: Vec<u8>,
    exceeded: bool,
}

fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    stream: GitOutputStream,
    limit_sender: Sender<GitOutputStream>,
) -> io::Result<BoundedOutput> {
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut exceeded = false;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        let retained = read.min(remaining);
        bytes.extend_from_slice(&buffer[..retained]);
        if retained < read && !exceeded {
            exceeded = true;
            let _ = limit_sender.send(stream);
        }
    }
    Ok(BoundedOutput { bytes, exceeded })
}

fn wait_for_exit(
    child: &mut Child,
    timeout: Option<Duration>,
    limit_receiver: &Receiver<GitOutputStream>,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<ExitStatus, GitRunError> {
    let started = Instant::now();
    loop {
        if let Ok(stream) = limit_receiver.try_recv() {
            let limit = match stream {
                GitOutputStream::Stdout => stdout_limit,
                GitOutputStream::Stderr => stderr_limit,
            };
            return Err(GitRunError::OutputLimitExceeded { stream, limit });
        }
        if let Some(status) = child.try_wait().map_err(GitRunError::Io)? {
            return Ok(status);
        }
        if let Some(timeout) = timeout
            && started.elapsed() >= timeout
        {
            return Err(GitRunError::TimedOut { timeout });
        }
        let delay = timeout
            .map(|timeout| WAIT_POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())))
            .unwrap_or(WAIT_POLL_INTERVAL);
        thread::sleep(delay);
    }
}

fn join_io_thread<T>(handle: thread::JoinHandle<io::Result<T>>) -> Result<T, GitRunError> {
    handle
        .join()
        .map_err(|_| GitRunError::Io(io::Error::other("git I/O worker panicked")))?
        .map_err(GitRunError::Io)
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn repository_path_is_one_argv_value_even_with_shell_metacharacters() {
        let root = tempdir().expect("temporary directory");
        let repository = root.path().join("repo;touch should-not-exist");
        fs::create_dir(&repository).expect("repository directory");
        let init = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repository)
            .status()
            .expect("git init");
        assert!(init.success());

        let output = GitRunner::default()
            .execute(&repository, &["rev-parse", "--is-inside-work-tree"])
            .expect("git command succeeds");

        assert_eq!(output.stdout, b"true\n");
        assert!(!root.path().join("should-not-exist").exists());
    }

    #[test]
    fn unsuccessful_exit_includes_stderr() {
        let directory = tempdir().expect("temporary directory");
        let error = GitRunner::default()
            .execute(directory.path(), &["rev-parse", "--is-inside-work-tree"])
            .expect_err("not a repository");

        match error {
            GitRunError::Unsuccessful { code, stderr } => {
                assert_ne!(code, Some(0));
                assert!(stderr.contains("not a git repository"));
            }
            error => panic!("unexpected failure: {error}"),
        }
    }

    #[test]
    fn legacy_executor_rejects_commands_that_are_not_guaranteed_read_only() {
        let error = GitRunner::default()
            .execute(Path::new("."), &["commit", "-m", "must not run"])
            .expect_err("legacy executor is read-only");

        assert!(matches!(
            error,
            GitRunError::ReadOnlyPolicyViolation { command } if command == "commit"
        ));
    }

    #[test]
    fn read_only_policy_rejects_output_and_external_helper_flags() {
        let oid = "0123456789012345678901234567890123456789";
        let forbidden = [
            vec!["log", "--output", "/tmp/history"],
            vec!["log", "--output=/tmp/history"],
            vec!["show", "--output=/tmp/details", oid, "--"],
            vec!["diff", "--ext-diff", "--"],
            vec!["diff", "--textconv", "--"],
            vec!["show", "--ext-diff", oid, "--"],
            vec!["show", "--textconv", oid, "--"],
            vec![
                "show",
                "--format=",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=2147483647",
                oid,
                "--",
                "plain/path.txt",
            ],
            vec![
                "show",
                "--format=",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=2147483647",
                oid,
                "--",
                ":(literal)",
            ],
            vec![
                "show",
                "--format=",
                "--no-ext-diff",
                "--textconv",
                "--unified=2147483647",
                oid,
                "--",
                ":(literal)file.txt",
            ],
            vec!["show", oid, "--"],
            vec!["show", "--format=", oid, "--"],
            vec!["log", "-c", "diff.external=unsafe", oid],
            vec!["config", "--get", "diff.external.command"],
        ];

        for arguments in forbidden {
            let arguments = arguments
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            assert!(matches!(
                validate_read_only_command(&arguments),
                Err(GitRunError::ReadOnlyPolicyViolation { .. })
            ));
        }
    }

    #[test]
    fn read_only_policy_preserves_fixed_status_metadata_history_and_navigation_queries() {
        let oid = "0123456789012345678901234567890123456789";
        let allowed = [
            vec![
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
            ],
            vec!["symbolic-ref", "--quiet", "--short", "HEAD"],
            vec!["config", "--get", "branch.feature/topic.remote"],
            vec!["remote", "get-url", "--push", "--", "origin"],
            vec![
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads",
                "refs/remotes",
            ],
            vec!["worktree", "list", "--porcelain", "-z"],
            vec![
                "log",
                "-z",
                "--topo-order",
                "--date-order",
                "--max-count=101",
                "--skip=0",
                "--format=%H",
                oid,
            ],
            vec!["log", "-g", "--format=%H", "refs/stash"],
            vec!["show", "-s", "-z", "--format=%H", oid, "--"],
            vec![
                "show",
                "--format=",
                "--first-parent",
                "--name-status",
                "-z",
                "-M",
                oid,
                "--",
            ],
            vec![
                "show",
                "--format=",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=2147483647",
                oid,
                "--",
                ":(literal)--output=/tmp/unsafe :(glob)*",
            ],
        ];

        for arguments in allowed {
            let arguments = arguments
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            validate_read_only_command(&arguments).expect("known read-only query is allowed");
        }
    }

    #[test]
    fn stash_inspection_allowlist_accepts_only_fixed_first_parent_shapes() {
        let oid = "0123456789012345678901234567890123456789";
        let allowed = [
            vec![
                "show",
                "--format=",
                "--first-parent",
                "--no-ext-diff",
                "--no-textconv",
                "--name-status",
                "-z",
                "-M",
                oid,
                "--",
            ],
            vec![
                "show",
                "--format=",
                "--first-parent",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=2147483647",
                oid,
                "--",
                ":(literal)old name.txt",
                ":(literal)new name.txt",
            ],
        ];
        for arguments in allowed {
            let arguments = arguments
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            validate_read_only_command(&arguments).expect("fixed stash query is read-only");
        }

        let forbidden = [
            vec![
                "show",
                "--format=",
                "--first-parent",
                "--no-ext-diff",
                "--no-color",
                "-M",
                "--unified=2147483647",
                oid,
                "--",
                ":(literal)file.txt",
            ],
            vec![
                "show",
                "--format=",
                "--first-parent",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=2147483647",
                "abc",
                "--",
                ":(literal)file.txt",
            ],
            vec![
                "show",
                "--format=",
                "--first-parent",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                "--unified=2147483647",
                oid,
                "--",
                "file.txt",
            ],
        ];
        for arguments in forbidden {
            let arguments = arguments
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            assert!(matches!(
                validate_read_only_command(&arguments),
                Err(GitRunError::ReadOnlyPolicyViolation { .. })
            ));
        }
    }

    #[test]
    fn working_tree_diff_allowlist_accepts_only_the_bounded_literal_shape() {
        let allowed = [
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "--unified=2147483647",
            "HEAD",
            "--",
            ":(literal)--output=/tmp/pwn :(glob)*",
        ]
        .map(OsString::from);
        validate_read_only_command(&allowed).expect("exact working tree diff is allowed");
        let cached_allowed = [
            "diff",
            "--cached",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "--unified=2147483647",
            "HEAD",
            "--",
            ":(literal)same.txt",
        ]
        .map(OsString::from);
        validate_read_only_command(&cached_allowed)
            .expect("exact cached collision diff is allowed");

        for rejected in [
            vec![
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "-M",
                "--unified=2147483647",
                "HEAD",
                "--",
                "--output=/tmp/pwn",
            ],
            vec![
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "-M",
                "--unified=3",
                "HEAD",
                "--",
                ":(literal)file.txt",
            ],
            vec![
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--textconv",
                "-M",
                "--unified=2147483647",
                "HEAD",
                "--",
                ":(literal)file.txt",
            ],
            vec![
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "-M",
                "--unified=2147483647",
                "--cached",
                "--",
                ":(literal)file.txt",
            ],
        ] {
            let arguments = rejected.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert!(matches!(
                validate_read_only_command(&arguments),
                Err(GitRunError::ReadOnlyPolicyViolation { .. })
            ));
        }
    }

    #[cfg(unix)]
    mod unix {
        use std::{os::unix::fs::PermissionsExt, time::Duration};

        use super::*;

        fn executable_script(contents: &str) -> (tempfile::TempDir, GitRunner) {
            let directory = tempdir().expect("temporary directory");
            let script = directory.path().join("fake-git");
            fs::write(&script, contents).expect("write script");
            fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
                .expect("make executable");
            (directory, GitRunner::new(script))
        }

        #[test]
        fn invocation_owns_argv_and_forwards_stdin_without_a_shell() {
            let (_directory, runner) =
                executable_script("#!/bin/sh\nprintf '%s\\n' \"$@\"\nprintf 'stdin='\ncat\n");
            let mut argument = String::from("value; untouched");
            let invocation = GitInvocation::new(
                GitInvocationPolicy::Mutating,
                [OsString::from("commit"), OsString::from(argument.as_str())],
            )
            .with_stdin(b"commit message".to_vec());
            argument.clear();

            let output = runner
                .run(Path::new("repo path"), invocation)
                .expect("fake git succeeds");
            let stdout = String::from_utf8(output.stdout).expect("UTF-8 output");

            assert!(stdout.contains("repo path"));
            assert!(stdout.contains("value; untouched"));
            assert!(stdout.ends_with("stdin=commit message"));
        }

        #[test]
        fn optional_locks_are_disabled_only_for_read_only_invocations() {
            let (_directory, runner) =
                executable_script("#!/bin/sh\nprintf '%s' \"${GIT_OPTIONAL_LOCKS-unset}\"\n");
            // SAFETY: this crate's tests do not concurrently read this variable.
            unsafe { std::env::set_var("GIT_OPTIONAL_LOCKS", "inherited") };

            let read_only = runner
                .run(
                    Path::new("."),
                    GitInvocation::new(GitInvocationPolicy::ReadOnly, ["status"]),
                )
                .expect("read-only command");
            let mutating = runner
                .run(
                    Path::new("."),
                    GitInvocation::new(GitInvocationPolicy::Mutating, ["commit"]),
                )
                .expect("mutating command");
            let network = runner
                .run(
                    Path::new("."),
                    GitInvocation::new(GitInvocationPolicy::Network, ["fetch"]),
                )
                .expect("network command");

            // SAFETY: restores the variable before this test ends.
            unsafe { std::env::remove_var("GIT_OPTIONAL_LOCKS") };
            assert_eq!(read_only.stdout, b"0");
            assert_eq!(mutating.stdout, b"unset");
            assert_eq!(network.stdout, b"unset");
        }

        #[test]
        fn rejects_output_over_the_configured_limit() {
            let (_directory, runner) =
                executable_script("#!/bin/sh\nprintf '123456'\nprintf 'abcdef' >&2\n");
            let error = runner
                .run(
                    Path::new("."),
                    GitInvocation::new(GitInvocationPolicy::ReadOnly, ["status"])
                        .with_output_limits(5, 10),
                )
                .expect_err("stdout is too large");

            assert!(matches!(
                error,
                GitRunError::OutputLimitExceeded {
                    stream: GitOutputStream::Stdout,
                    limit: 5
                }
            ));
        }

        #[test]
        fn terminates_infinite_streaming_output_as_soon_as_the_limit_is_exceeded() {
            let (_directory, runner) =
                executable_script("#!/bin/sh\nwhile :; do printf '0123456789'; done\n");
            let started = Instant::now();
            let error = runner
                .run(
                    Path::new("."),
                    GitInvocation::new(GitInvocationPolicy::ReadOnly, ["status"])
                        .with_output_limits(64, 64),
                )
                .expect_err("streaming stdout exceeds its limit");

            assert!(matches!(
                error,
                GitRunError::OutputLimitExceeded {
                    stream: GitOutputStream::Stdout,
                    limit: 64
                }
            ));
            assert!(started.elapsed() < Duration::from_secs(2));
        }

        #[test]
        fn terminates_a_command_after_its_timeout() {
            let (_directory, runner) = executable_script("#!/bin/sh\nwhile :; do :; done\n");
            let timeout = Duration::from_millis(30);
            let started = Instant::now();
            let error = runner
                .run(
                    Path::new("."),
                    GitInvocation::new(GitInvocationPolicy::Network, ["fetch"])
                        .with_timeout(timeout),
                )
                .expect_err("command times out");

            assert!(matches!(error, GitRunError::TimedOut { timeout: value } if value == timeout));
            assert!(started.elapsed() < Duration::from_secs(1));
        }

        #[test]
        fn timeout_is_not_held_open_by_a_descendant_inheriting_output_pipes() {
            let (_directory, runner) = executable_script("#!/bin/sh\nsleep 30 &\nwait\n");
            let timeout = Duration::from_millis(30);
            let started = Instant::now();
            let error = runner
                .run(
                    Path::new("."),
                    GitInvocation::new(GitInvocationPolicy::Network, ["fetch"])
                        .with_timeout(timeout),
                )
                .expect_err("the process group times out");

            assert!(matches!(error, GitRunError::TimedOut { timeout: value } if value == timeout));
            assert!(started.elapsed() < Duration::from_secs(1));
        }
    }
}
