use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::{AppState, CommandError};

const DEFAULT_MAX_LOG_KILOBYTES: u64 = 256;
const MIN_MAX_LOG_KILOBYTES: u64 = 64;
const MAX_MAX_LOG_KILOBYTES: u64 = 8 * 1024;
const MAX_VISIBLE_ENTRIES: usize = 1_000;
const MAX_FIELD_LENGTH: usize = 128;
const LOG_FILE_NAME: &str = "diagnostics.jsonl";
const ROTATED_LOG_FILE_NAME: &str = "diagnostics.1.jsonl";
const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone)]
pub(crate) struct DiagnosticsState {
    inner: Arc<Mutex<DiagnosticsStore>>,
}

#[derive(Debug)]
struct DiagnosticsStore {
    root: PathBuf,
    pointer_path: PathBuf,
    max_log_kilobytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsSettingsResponse {
    data_directory: String,
    max_log_kilobytes: u64,
    log_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticEntry {
    timestamp_ms: u64,
    severity: DiagnosticSeverity,
    subsystem: String,
    event_code: String,
    message: String,
    fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticLogResponse {
    entries: Vec<DiagnosticEntry>,
    total_bytes: u64,
    truncated: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DataRootPointer {
    version: u8,
    data_directory: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSettings {
    version: u8,
    diagnostics: PersistedDiagnosticsSettings,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedDiagnosticsSettings {
    max_log_kilobytes: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SelectedDirectoryResponse {
    path: Option<String>,
}

impl DiagnosticsState {
    pub(crate) fn open(default_root: PathBuf, pointer_path: PathBuf) -> io::Result<Self> {
        let root = read_pointer(&pointer_path)
            .filter(|path| path.is_absolute() && path.is_dir())
            .unwrap_or(default_root);
        fs::create_dir_all(&root)?;
        let max_log_kilobytes = read_settings(&root)
            .map(|settings| validated_log_limit(settings.diagnostics.max_log_kilobytes))
            .unwrap_or(DEFAULT_MAX_LOG_KILOBYTES);
        let state = Self {
            inner: Arc::new(Mutex::new(DiagnosticsStore {
                root,
                pointer_path,
                max_log_kilobytes,
            })),
        };
        state.persist_current_settings()?;
        Ok(state)
    }

    pub(crate) fn record_ai_event(
        &self,
        severity: &'static str,
        event_code: &'static str,
        message: &'static str,
        provider: &'static str,
    ) {
        let severity = match severity {
            "error" => DiagnosticSeverity::Error,
            "warning" => DiagnosticSeverity::Warning,
            _ => DiagnosticSeverity::Info,
        };
        let fields = BTreeMap::from([("provider".to_owned(), sanitize_field(provider))]);
        let entry = DiagnosticEntry {
            timestamp_ms: now_millis(),
            severity,
            subsystem: "aiSupport".to_owned(),
            event_code: event_code.to_owned(),
            message: message.to_owned(),
            fields,
        };
        if let Ok(mut store) = self.inner.lock() {
            let _ = store.append(&entry);
        }
    }

    pub(crate) fn data_root(&self) -> Result<PathBuf, CommandError> {
        let store = self.inner.lock().map_err(|_| unavailable())?;
        Ok(store.root.clone())
    }

    fn settings(&self) -> Result<DiagnosticsSettingsResponse, CommandError> {
        let store = self.inner.lock().map_err(|_| unavailable())?;
        Ok(store.settings_response())
    }

    fn configure(
        &self,
        data_directory: String,
        max_log_kilobytes: u64,
    ) -> Result<DiagnosticsSettingsResponse, CommandError> {
        let requested = PathBuf::from(data_directory);
        if !requested.is_absolute() {
            return Err(command_error(
                "diagnostics directory must be an absolute path",
            ));
        }
        let root = fs::canonicalize(&requested)
            .map_err(|_| command_error("diagnostics directory is unavailable"))?;
        if !root.is_dir() {
            return Err(command_error(
                "diagnostics directory must be an existing folder",
            ));
        }
        let max_log_kilobytes = validate_requested_log_limit(max_log_kilobytes)?;
        let mut store = self.inner.lock().map_err(|_| unavailable())?;
        let next_settings = PersistedSettings {
            version: 1,
            diagnostics: PersistedDiagnosticsSettings { max_log_kilobytes },
        };
        write_json_atomic(&root.join(SETTINGS_FILE_NAME), &next_settings)
            .map_err(|_| command_error("diagnostics settings could not be written"))?;
        let pointer = DataRootPointer {
            version: 1,
            data_directory: root.to_string_lossy().into_owned(),
        };
        write_json_atomic(&store.pointer_path, &pointer)
            .map_err(|_| command_error("diagnostics directory preference could not be written"))?;
        store.root = root;
        store.max_log_kilobytes = max_log_kilobytes;
        Ok(store.settings_response())
    }

    fn read(&self) -> Result<DiagnosticLogResponse, CommandError> {
        let store = self.inner.lock().map_err(|_| unavailable())?;
        store
            .read()
            .map_err(|_| command_error("diagnostic log could not be read"))
    }

    fn clear(&self) -> Result<(), CommandError> {
        let store = self.inner.lock().map_err(|_| unavailable())?;
        for path in [store.log_path(), store.rotated_log_path()] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => return Err(command_error("diagnostic log could not be cleared")),
            }
        }
        Ok(())
    }

    fn persist_current_settings(&self) -> io::Result<()> {
        let store = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("lock poisoned"))?;
        write_json_atomic(
            &store.root.join(SETTINGS_FILE_NAME),
            &PersistedSettings {
                version: 1,
                diagnostics: PersistedDiagnosticsSettings {
                    max_log_kilobytes: store.max_log_kilobytes,
                },
            },
        )
    }
}

impl DiagnosticsStore {
    fn settings_response(&self) -> DiagnosticsSettingsResponse {
        DiagnosticsSettingsResponse {
            data_directory: self.root.to_string_lossy().into_owned(),
            max_log_kilobytes: self.max_log_kilobytes,
            log_file: self.log_path().to_string_lossy().into_owned(),
        }
    }

    fn log_path(&self) -> PathBuf {
        self.root.join(LOG_FILE_NAME)
    }

    fn rotated_log_path(&self) -> PathBuf {
        self.root.join(ROTATED_LOG_FILE_NAME)
    }

    fn append(&mut self, entry: &DiagnosticEntry) -> io::Result<()> {
        let mut encoded = serde_json::to_vec(entry).map_err(io::Error::other)?;
        encoded.push(b'\n');
        let max_bytes = self.max_log_kilobytes * 1024;
        let max_file_bytes = max_bytes / 2;
        if encoded.len() as u64 > max_file_bytes {
            return Ok(());
        }
        let log_path = self.log_path();
        ensure_regular_or_missing(&log_path)?;
        ensure_regular_or_missing(&self.rotated_log_path())?;
        let current_size = fs::metadata(&log_path)
            .map(|value| value.len())
            .unwrap_or(0);
        if current_size.saturating_add(encoded.len() as u64) > max_file_bytes {
            let rotated = self.rotated_log_path();
            let _ = fs::remove_file(&rotated);
            if log_path.exists() {
                fs::rename(&log_path, rotated)?;
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        file.write_all(&encoded)?;
        file.flush()
    }

    fn read(&self) -> io::Result<DiagnosticLogResponse> {
        let paths = [self.rotated_log_path(), self.log_path()];
        let total_bytes = paths
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        let mut entries = Vec::new();
        let mut malformed_or_omitted = false;
        for path in paths {
            ensure_regular_or_missing(&path)?;
            let file = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            for line in BufReader::new(file).lines() {
                match line.and_then(|line| {
                    serde_json::from_str::<DiagnosticEntry>(&line).map_err(io::Error::other)
                }) {
                    Ok(entry) => entries.push(entry),
                    Err(_) => malformed_or_omitted = true,
                }
            }
        }
        if entries.len() > MAX_VISIBLE_ENTRIES {
            let remove = entries.len() - MAX_VISIBLE_ENTRIES;
            entries.drain(..remove);
            malformed_or_omitted = true;
        }
        entries.reverse();
        Ok(DiagnosticLogResponse {
            entries,
            total_bytes,
            truncated: malformed_or_omitted,
        })
    }
}

#[tauri::command]
pub(crate) fn diagnostics_settings(
    state: State<'_, AppState>,
) -> Result<DiagnosticsSettingsResponse, CommandError> {
    state.diagnostics.settings()
}

#[tauri::command]
pub(crate) fn diagnostics_update_settings(
    data_directory: String,
    max_log_kilobytes: u64,
    state: State<'_, AppState>,
) -> Result<DiagnosticsSettingsResponse, CommandError> {
    state
        .diagnostics
        .configure(data_directory, max_log_kilobytes)
}

#[tauri::command]
pub(crate) fn diagnostics_read(
    state: State<'_, AppState>,
) -> Result<DiagnosticLogResponse, CommandError> {
    state.diagnostics.read()
}

#[tauri::command]
pub(crate) fn diagnostics_clear(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.diagnostics.clear()
}

#[tauri::command]
pub(crate) async fn select_diagnostics_directory(
    app: AppHandle,
    initial_path: Option<String>,
) -> Result<SelectedDirectoryResponse, CommandError> {
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Select Skibidibi Git data directory");
    if let Some(path) = initial_path.filter(|path| Path::new(path).is_dir()) {
        dialog = dialog.set_directory(path);
    }
    let path = dialog
        .blocking_pick_folder()
        .map(|path| {
            path.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|_| command_error("selected diagnostics directory is invalid"))
        })
        .transpose()?;
    Ok(SelectedDirectoryResponse { path })
}

fn read_pointer(path: &Path) -> Option<PathBuf> {
    let bytes = fs::read(path).ok()?;
    let pointer = serde_json::from_slice::<DataRootPointer>(&bytes).ok()?;
    (pointer.version == 1).then(|| PathBuf::from(pointer.data_directory))
}

fn read_settings(root: &Path) -> Option<PersistedSettings> {
    let bytes = fs::read(root.join(SETTINGS_FILE_NAME)).ok()?;
    let settings = serde_json::from_slice::<PersistedSettings>(&bytes).ok()?;
    (settings.version == 1).then_some(settings)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("settings path has no parent"))?;
    fs::create_dir_all(parent)?;
    ensure_regular_or_missing(path)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
}

fn validated_log_limit(value: u64) -> u64 {
    value.clamp(MIN_MAX_LOG_KILOBYTES, MAX_MAX_LOG_KILOBYTES)
}

fn validate_requested_log_limit(value: u64) -> Result<u64, CommandError> {
    if (MIN_MAX_LOG_KILOBYTES..=MAX_MAX_LOG_KILOBYTES).contains(&value) {
        Ok(value)
    } else {
        Err(command_error(
            "diagnostic log limit must be between 64 and 8192 KiB",
        ))
    }
}

fn sanitize_field(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_FIELD_LENGTH)
        .collect()
}

fn ensure_regular_or_missing(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(io::Error::other("diagnostic target is not a regular file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn unavailable() -> CommandError {
    command_error("diagnostics are temporarily unavailable")
}

fn command_error(message: impl Into<String>) -> CommandError {
    CommandError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_bounded_jsonl_and_returns_newest_entries_first() {
        let temporary = tempfile::tempdir().unwrap();
        let pointer = temporary.path().join("config/root.json");
        let root = temporary.path().join("data");
        let state = DiagnosticsState::open(root, pointer).unwrap();
        state
            .configure(
                temporary.path().join("data").to_string_lossy().into_owned(),
                MIN_MAX_LOG_KILOBYTES,
            )
            .unwrap();
        for index in 0..900 {
            state.record_ai_event(
                "warning",
                "failed",
                "Generation failed",
                if index % 2 == 0 { "codex" } else { "cursor" },
            );
        }
        let response = state.read().unwrap();
        assert!(!response.entries.is_empty());
        assert!(response.total_bytes <= MIN_MAX_LOG_KILOBYTES * 1024);
        assert_eq!(response.entries[0].event_code, "failed");
    }

    #[test]
    fn settings_and_custom_root_are_persisted_without_log_payloads() {
        let temporary = tempfile::tempdir().unwrap();
        let pointer = temporary.path().join("config/root.json");
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        fs::create_dir_all(&second).unwrap();
        let state = DiagnosticsState::open(first, pointer.clone()).unwrap();
        state
            .configure(second.to_string_lossy().into_owned(), 512)
            .unwrap();
        let reopened = DiagnosticsState::open(temporary.path().join("fallback"), pointer).unwrap();
        let settings = reopened.settings().unwrap();
        assert_eq!(settings.max_log_kilobytes, 512);
        assert_eq!(
            PathBuf::from(settings.data_directory),
            second.canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_follow_a_log_symlink() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("data");
        let state = DiagnosticsState::open(root.clone(), temporary.path().join("config/root.json"))
            .unwrap();
        let target = temporary.path().join("must-not-change");
        fs::write(&target, b"original").unwrap();
        symlink(&target, root.join(LOG_FILE_NAME)).unwrap();

        state.record_ai_event("error", "failed", "Generation failed", "codex");
        assert_eq!(fs::read(target).unwrap(), b"original");
        assert!(state.read().is_err());
    }
}
