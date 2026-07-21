use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use crate::{
    GitHubMethod, GitHubRequest, GitHubResponse, GitHubTransport, PersonalAccessToken,
    TransportError,
};

const OUTPUT_LIMIT: usize = 8 * 1024 * 1024 + 64 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// GitHub API transport backed by the user's authenticated `gh` session. The credential never
/// leaves GitHub CLI: this process receives only API request bodies and response JSON.
#[derive(Debug, Clone)]
pub struct GitHubCliTransport {
    executable: Option<PathBuf>,
    allowed_origin: Url,
    timeout: Duration,
}

impl GitHubCliTransport {
    pub fn discover(allowed_origin: Url) -> Self {
        Self {
            executable: discover_executable(),
            allowed_origin,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_executable(executable: impl Into<PathBuf>, allowed_origin: Url) -> Self {
        Self {
            executable: Some(executable.into()),
            allowed_origin,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    fn same_origin(&self, target: &Url) -> bool {
        target.scheme() == self.allowed_origin.scheme()
            && target.host_str() == self.allowed_origin.host_str()
            && target.port_or_known_default() == self.allowed_origin.port_or_known_default()
            && target.username().is_empty()
            && target.password().is_none()
    }

    fn execute_inner(&self, request: GitHubRequest) -> Result<GitHubResponse, TransportError> {
        if !self.same_origin(&request.url) {
            return Err(TransportError::UnsafeTarget);
        }
        if !request.headers.is_empty() {
            return Err(TransportError::InvalidHeader);
        }
        let executable = self
            .executable
            .as_ref()
            .ok_or(TransportError::InvalidCredential)?;
        let mut command = tokio::process::Command::new(executable);
        command
            .args([
                "api",
                request.url.as_str(),
                "--hostname",
                "github.com",
                "--include",
                "--method",
                match request.method {
                    GitHubMethod::Get => "GET",
                    GitHubMethod::Post => "POST",
                },
            ])
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1")
            .stdin(if request.body.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if request.body.is_some() {
            command.args(["--input", "-"]);
        }
        configure_process_tree(&mut command);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|_| TransportError::Network)?;
        let timeout = self.timeout;
        let process_id = Arc::new(AtomicU32::new(0));
        let operation_process_id = Arc::clone(&process_id);
        let operation = async move {
            let mut child = command
                .spawn()
                .map_err(|_| TransportError::InvalidCredential)?;
            operation_process_id.store(child.id().unwrap_or(0), Ordering::Release);
            if let Some(body) = request.body {
                let mut stdin = child.stdin.take().ok_or(TransportError::Network)?;
                stdin
                    .write_all(&body)
                    .await
                    .map_err(|_| TransportError::Network)?;
                stdin
                    .shutdown()
                    .await
                    .map_err(|_| TransportError::Network)?;
            }
            let stdout = child.stdout.take().ok_or(TransportError::Network)?;
            let mut bytes = Vec::with_capacity(16 * 1024);
            let mut bounded_stdout = stdout.take((OUTPUT_LIMIT + 1) as u64);
            let read = bounded_stdout.read_to_end(&mut bytes);
            let (status, read_result) = tokio::join!(child.wait(), read);
            let status = status.map_err(|_| TransportError::Network)?;
            read_result.map_err(|_| TransportError::Network)?;
            if bytes.len() > OUTPUT_LIMIT {
                return Err(TransportError::ResponseTooLarge);
            }
            match parse_included_response(&bytes) {
                Ok(response) => Ok(response),
                Err(_) if !status.success() => Ok(authentication_required_response()),
                Err(error) => Err(error),
            }
        };
        match runtime.block_on(async { tokio::time::timeout(timeout, operation).await }) {
            Ok(result) => result,
            Err(_) => {
                terminate_process_tree(process_id.load(Ordering::Acquire));
                Err(TransportError::TimedOut)
            }
        }
    }
}

#[cfg(unix)]
fn configure_process_tree(command: &mut tokio::process::Command) {
    use std::os::unix::process::CommandExt;
    command.as_std_mut().process_group(0);
}

#[cfg(windows)]
fn configure_process_tree(command: &mut tokio::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command
        .as_std_mut()
        .creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn configure_process_tree(_command: &mut tokio::process::Command) {}

#[cfg(unix)]
fn terminate_process_tree(process_id: u32) {
    if process_id == 0 {
        return;
    }
    // SAFETY: the direct child is created as the leader of a fresh process group. This executes
    // only after the bounded API invocation timed out and complements Tokio's direct-child kill.
    unsafe {
        libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_process_tree(process_id: u32) {
    if process_id == 0 {
        return;
    }
    let system_root = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let _ = std::process::Command::new(system_root.join("System32").join("taskkill.exe"))
        .args(["/PID", &process_id.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(_process_id: u32) {}

impl GitHubTransport for GitHubCliTransport {
    fn execute(
        &self,
        request: GitHubRequest,
        _credential: &PersonalAccessToken,
    ) -> Result<GitHubResponse, TransportError> {
        self.execute_inner(request)
    }
}

fn parse_included_response(bytes: &[u8]) -> Result<GitHubResponse, TransportError> {
    let separator = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        })
        .ok_or(TransportError::Network)?;
    let header_text =
        std::str::from_utf8(&bytes[..separator.0]).map_err(|_| TransportError::Network)?;
    let mut lines = header_text.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .filter(|status| (100..=599).contains(status))
        .ok_or(TransportError::Network)?;
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(TransportError::InvalidHeader)?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name.is_empty()
            || value.chars().any(char::is_control)
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(TransportError::InvalidHeader);
        }
        headers.insert(name, value.to_owned());
    }
    Ok(GitHubResponse {
        status,
        headers,
        body: bytes[separator.0 + separator.1..].to_vec(),
    })
}

fn authentication_required_response() -> GitHubResponse {
    GitHubResponse {
        status: 401,
        headers: BTreeMap::new(),
        body: br#"{"message":"GitHub CLI authentication is required"}"#.to_vec(),
    }
}

fn discover_executable() -> Option<PathBuf> {
    if let Some(path) = env::var_os("GH_PATH").map(PathBuf::from)
        && trusted_explicit_path(&path)
    {
        return Some(path);
    }
    platform_candidates()
        .into_iter()
        .chain(path_candidates())
        .find(|path| trusted_explicit_path(path))
}

fn trusted_explicit_path(path: &Path) -> bool {
    path.is_absolute() && path.is_file()
}

fn path_candidates() -> impl Iterator<Item = PathBuf> {
    let executable_name = if cfg!(windows) { "gh.exe" } else { "gh" };
    env::var_os("PATH")
        .into_iter()
        .flat_map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .filter(|directory| directory.is_absolute())
        .map(move |directory| directory.join(executable_name))
}

#[cfg(target_os = "macos")]
fn platform_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin/gh"),
        PathBuf::from("/usr/local/bin/gh"),
    ]
}

#[cfg(target_os = "windows")]
fn platform_candidates() -> Vec<PathBuf> {
    env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .map(|root| vec![root.join("GitHub CLI").join("gh.exe")])
        .unwrap_or_default()
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn platform_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/bin/gh"),
        PathBuf::from("/usr/bin/gh"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt, time::Instant};

    fn origin() -> Url {
        Url::parse("https://api.github.com/").unwrap()
    }

    fn request() -> GitHubRequest {
        GitHubRequest {
            method: GitHubMethod::Get,
            url: Url::parse("https://api.github.com/user").unwrap(),
            headers: BTreeMap::new(),
            body: None,
        }
    }

    #[test]
    fn parses_status_headers_and_json_body() {
        let response = parse_included_response(
            b"HTTP/2.0 200 OK\nX-RateLimit-Remaining: 4999\nLink: <next>; rel=\"next\"\n\n{\"login\":\"octocat\"}",
        )
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.headers["x-ratelimit-remaining"], "4999");
        assert_eq!(response.body, br#"{"login":"octocat"}"#);
    }

    #[cfg(unix)]
    #[test]
    fn invokes_only_the_fixed_api_shape_without_a_token_command() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("gh");
        fs::write(
            &executable,
            "#!/bin/sh\ncase \"$*\" in\n  *'auth token'*) exit 9;;\nesac\n[ \"$1\" = api ] || exit 8\nprintf 'HTTP/2.0 200 OK\\nContent-Type: application/json\\n\\n{\"login\":\"octocat\",\"id\":7}'\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let transport = GitHubCliTransport::with_executable(executable, origin());

        let response = transport.execute_inner(request()).unwrap();

        assert_eq!(response.status, 200);
        assert!(
            String::from_utf8(response.body)
                .unwrap()
                .contains("octocat")
        );
    }

    #[cfg(unix)]
    #[test]
    fn descendant_holding_stdout_is_terminated_with_its_process_group() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("gh");
        let descendant_pid_path = directory.path().join("gh.pid");
        let script = format!(
            "#!/usr/bin/perl\nmy $pid=fork();\nexit 2 unless defined $pid;\nif ($pid==0) {{ sleep 3; exit 0; }}\nopen(my $file, '>', '{}') or exit 3; print $file $pid; close($file);\nprint \"HTTP/2.0 200 OK\\n\\n{{}}\";\nexit 0;\n",
            descendant_pid_path.display()
        );
        fs::write(&executable, script).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut transport = GitHubCliTransport::with_executable(executable, origin());
        transport.timeout = Duration::from_secs(2);
        let started = Instant::now();

        assert!(matches!(
            transport.execute_inner(request()),
            Err(TransportError::TimedOut)
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        let descendant_pid = fs::read_to_string(descendant_pid_path)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let mut terminated = false;
        for _ in 0..20 {
            // SAFETY: signal 0 only probes whether the test-owned process is still present.
            if unsafe { libc::kill(descendant_pid, 0) } != 0 {
                terminated = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            terminated,
            "descendant process survived the transport timeout"
        );
    }

    #[test]
    fn rejects_cross_origin_and_relative_executable_candidates() {
        let transport = GitHubCliTransport::discover(origin());
        let mut unsafe_request = request();
        unsafe_request.url = Url::parse("https://example.com/user").unwrap();
        assert!(matches!(
            transport.execute_inner(unsafe_request),
            Err(TransportError::UnsafeTarget)
        ));
        assert!(!trusted_explicit_path(Path::new("GitHub CLI/gh.exe")));
        assert!(
            discover_executable()
                .as_ref()
                .is_none_or(|path| path.is_absolute())
        );
    }
}
