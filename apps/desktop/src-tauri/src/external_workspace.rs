use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use repo_runtime::ManagedWorktreeError;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::{AppState, CommandError, mutation_lock, resolve_repository_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExternalWorkspaceTarget {
    VsCode,
    Cursor,
    System,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenBranchWorkspaceRequest {
    repository_id: String,
    branch_full_name: String,
    expected_oid: String,
    target: ExternalWorkspaceTarget,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenBranchWorkspaceResponse {
    path: String,
    worktree_created: bool,
    target: ExternalWorkspaceTarget,
}

#[tauri::command]
pub(crate) async fn open_branch_workspace(
    request: OpenBranchWorkspaceRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<OpenBranchWorkspaceResponse, CommandError> {
    let editor = match request.target {
        ExternalWorkspaceTarget::VsCode | ExternalWorkspaceTarget::Cursor => {
            Some(discover_editor(request.target).ok_or_else(|| editor_not_found(request.target))?)
        }
        ExternalWorkspaceTarget::System => None,
    };
    let repository = resolve_repository_path(&request.repository_id, &state)?;
    let data_root = state.diagnostics.data_root()?;
    let runtime = state.repositories.clone();
    let mutation = mutation_lock(&state, &repository)?;
    let branch_full_name = request.branch_full_name;
    let expected_oid = request.expected_oid;

    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let _guard = mutation.lock().map_err(|_| {
            command_error("repositoryBusy", "repository mutation queue is unavailable")
        })?;
        runtime
            .prepare_branch_worktree(&repository, &data_root, &branch_full_name, &expected_oid)
            .map_err(managed_worktree_error)
    })
    .await
    .map_err(|error| {
        command_error(
            "worktreeTaskFailed",
            &format!("branch worktree task failed: {error}"),
        )
    })??;

    let path = prepared
        .path
        .to_str()
        .ok_or_else(|| {
            command_error(
                "unsupportedPath",
                "the worktree path cannot be represented by the desktop application",
            )
        })?
        .to_owned();
    open_workspace_path(&app, request.target, &prepared.path, editor.as_deref())?;
    Ok(OpenBranchWorkspaceResponse {
        path,
        worktree_created: prepared.created,
        target: request.target,
    })
}

fn managed_worktree_error(error: ManagedWorktreeError) -> CommandError {
    command_error(error.code(), &error.to_string())
}

fn command_error(code: &str, message: &str) -> CommandError {
    CommandError {
        message: format!("{code}: {message}"),
    }
}

fn open_workspace_path(
    app: &AppHandle,
    target: ExternalWorkspaceTarget,
    path: &Path,
    editor: Option<&Path>,
) -> Result<(), CommandError> {
    let canonical = fs::canonicalize(path).map_err(|_| {
        command_error(
            "worktreeUnavailable",
            "the selected worktree folder is unavailable",
        )
    })?;
    if !canonical.is_dir() {
        return Err(command_error(
            "worktreeUnavailable",
            "the selected worktree path is not a folder",
        ));
    }

    match target {
        ExternalWorkspaceTarget::System => {
            let path = canonical.to_str().ok_or_else(|| {
                command_error(
                    "unsupportedPath",
                    "the worktree path cannot be opened by the system file manager",
                )
            })?;
            app.opener().open_path(path, None::<&str>).map_err(|error| {
                command_error(
                    "systemOpenFailed",
                    &format!("the folder could not be opened in Finder or Explorer: {error}"),
                )
            })
        }
        ExternalWorkspaceTarget::VsCode | ExternalWorkspaceTarget::Cursor => {
            let executable = editor.ok_or_else(|| editor_not_found(target))?;
            spawn_editor(executable, &canonical).map_err(|error| {
                command_error(
                    "editorOpenFailed",
                    &format!("{} could not be started: {error}", editor_name(target)),
                )
            })
        }
    }
}

fn editor_not_found(target: ExternalWorkspaceTarget) -> CommandError {
    command_error(
        "editorUnavailable",
        &format!(
            "{} is not installed or its command is not available on PATH",
            editor_name(target)
        ),
    )
}

fn editor_name(target: ExternalWorkspaceTarget) -> &'static str {
    match target {
        ExternalWorkspaceTarget::VsCode => "Visual Studio Code",
        ExternalWorkspaceTarget::Cursor => "Cursor",
        ExternalWorkspaceTarget::System => "the system file manager",
    }
}

fn discover_editor(target: ExternalWorkspaceTarget) -> Option<PathBuf> {
    resolve_trusted_executable(editor_candidates(target))
}

fn resolve_trusted_executable(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find_map(|candidate| {
        if !candidate.is_absolute() {
            return None;
        }
        let canonical = fs::canonicalize(candidate).ok()?;
        let metadata = fs::metadata(&canonical).ok()?;
        if !metadata.is_file() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return None;
            }
        }
        Some(canonical)
    })
}

fn editor_candidates(target: ExternalWorkspaceTarget) -> Vec<PathBuf> {
    let executable_names: &[&str] = match target {
        ExternalWorkspaceTarget::VsCode => {
            if cfg!(windows) {
                &["Code.exe"]
            } else {
                &["code"]
            }
        }
        ExternalWorkspaceTarget::Cursor => {
            if cfg!(windows) {
                &["Cursor.exe", "cursor.exe"]
            } else {
                &["cursor"]
            }
        }
        ExternalWorkspaceTarget::System => return Vec::new(),
    };
    let mut candidates = platform_editor_candidates(target);
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
            candidates.extend(executable_names.iter().map(|name| directory.join(name)));
        }
    }
    candidates
}

#[cfg(target_os = "macos")]
fn platform_editor_candidates(target: ExternalWorkspaceTarget) -> Vec<PathBuf> {
    let application_paths: &[&str] = match target {
        ExternalWorkspaceTarget::VsCode => &[
            "Visual Studio Code.app/Contents/Resources/app/bin/code",
            "Visual Studio Code.app/Contents/MacOS/Electron",
        ],
        ExternalWorkspaceTarget::Cursor => &[
            "Cursor.app/Contents/Resources/app/bin/cursor",
            "Cursor.app/Contents/MacOS/Cursor",
        ],
        ExternalWorkspaceTarget::System => return Vec::new(),
    };
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join("Applications"));
    }
    roots
        .into_iter()
        .flat_map(|root| application_paths.iter().map(move |path| root.join(path)))
        .collect()
}

#[cfg(target_os = "windows")]
fn platform_editor_candidates(target: ExternalWorkspaceTarget) -> Vec<PathBuf> {
    let suffixes: &[&str] = match target {
        ExternalWorkspaceTarget::VsCode => &[
            "Programs/Microsoft VS Code/Code.exe",
            "Microsoft VS Code/Code.exe",
        ],
        ExternalWorkspaceTarget::Cursor => &["Programs/cursor/Cursor.exe", "Cursor/Cursor.exe"],
        ExternalWorkspaceTarget::System => return Vec::new(),
    };
    ["LOCALAPPDATA", "ProgramFiles"]
        .into_iter()
        .filter_map(|variable| std::env::var_os(variable).map(PathBuf::from))
        .flat_map(|root| suffixes.iter().map(move |suffix| root.join(suffix)))
        .collect()
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn platform_editor_candidates(target: ExternalWorkspaceTarget) -> Vec<PathBuf> {
    let name = match target {
        ExternalWorkspaceTarget::VsCode => "code",
        ExternalWorkspaceTarget::Cursor => "cursor",
        ExternalWorkspaceTarget::System => return Vec::new(),
    };
    ["/usr/local/bin", "/usr/bin", "/snap/bin"]
        .into_iter()
        .map(|root| Path::new(root).join(name))
        .collect()
}

fn editor_arguments(path: &Path) -> Vec<OsString> {
    vec![path.as_os_str().to_owned()]
}

fn spawn_editor(executable: &Path, path: &Path) -> std::io::Result<()> {
    let mut command = Command::new(executable);
    command
        .args(editor_arguments(path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure_detached_process(&mut command);
    command.spawn().map(|_| ())
}

#[cfg(windows)]
fn configure_detached_process(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
}

#[cfg(not(windows))]
fn configure_detached_process(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn editor_arguments_keep_the_workspace_path_as_one_argv_value() {
        let path = Path::new("/tmp/work tree;echo unsafe");
        assert_eq!(editor_arguments(path), vec![OsString::from(path)]);
    }

    #[cfg(unix)]
    #[test]
    fn executable_resolution_requires_an_absolute_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempdir().unwrap();
        let non_executable = fixture.path().join("not-executable");
        let executable = fixture.path().join("editor");
        fs::write(&non_executable, "fixture").unwrap();
        fs::write(&executable, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();

        let resolved = resolve_trusted_executable([
            PathBuf::from("relative-editor"),
            non_executable,
            executable.clone(),
        ]);

        assert_eq!(resolved, Some(executable.canonicalize().unwrap()));
    }

    #[test]
    fn system_target_never_resolves_an_editor_executable() {
        assert!(editor_candidates(ExternalWorkspaceTarget::System).is_empty());
    }
}
