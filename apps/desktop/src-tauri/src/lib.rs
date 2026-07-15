use std::path::Path;

use app_domain::RepositoryStatus;
use repo_runtime::{RepositoryRuntime, RepositoryRuntimeError};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Default)]
struct AppState {
    repositories: RepositoryRuntime,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandError {
    message: String,
}

impl From<RepositoryRuntimeError> for CommandError {
    fn from(error: RepositoryRuntimeError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

#[tauri::command]
async fn repository_status(
    repository_path: String,
    state: State<'_, AppState>,
) -> Result<RepositoryStatus, CommandError> {
    let repositories = state.repositories.clone();
    tauri::async_runtime::spawn_blocking(move || {
        repositories
            .status(Path::new(&repository_path))
            .map_err(CommandError::from)
    })
    .await
    .map_err(|error| CommandError {
        message: format!("repository status task failed: {error}"),
    })?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![repository_status])
        .run(tauri::generate_context!())
        .expect("error while running the desktop application");
}
