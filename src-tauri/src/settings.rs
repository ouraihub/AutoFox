//! Application settings boundary.

use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};

use crate::ipc::IpcError;
use tauri::Manager;

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, IpcError> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| IpcError::internal(error.to_string()))?;
    Ok(dir.join("settings.json"))
}

/// Parser selection policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractorPolicy {
    /// Choose according to configured backend order.
    Auto,
    /// Use only the bundled or configured yt-dlp backend.
    YtdlpOnly,
    /// Use only a native Rust backend.
    NativeOnly,
}

/// Application settings contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Settings schema version.
    pub schema_version: u32,
    /// Download destination chosen by the user.
    pub download_directory: Option<String>,
    /// Backend routing policy.
    pub extractor_policy: ExtractorPolicy,
    /// Whether PATH lookup is explicitly enabled.
    pub allow_path_lookup: bool,
    /// Optional user-selected yt-dlp executable.
    pub ytdlp_path: Option<String>,
    /// Optional user-selected ffmpeg executable.
    pub ffmpeg_path: Option<String>,
    /// Minimum backend log level displayed in the app.
    pub log_level: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            download_directory: None,
            extractor_policy: ExtractorPolicy::Auto,
            allow_path_lookup: false,
            ytdlp_path: None,
            ffmpeg_path: None,
            log_level: "INFO".to_owned(),
        }
    }
}
/// Shared in-memory settings state.
#[derive(Default)]
pub struct SettingsState(pub Mutex<Settings>);

/// Reads settings from disk, falling back to defaults on first launch.
#[tauri::command]
pub fn get_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, SettingsState>,
) -> Result<Settings, IpcError> {
    let path = settings_path(&app)?;
    let loaded = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .unwrap_or_default();
    state
        .0
        .lock()
        .map(|mut current| {
            *current = loaded.clone();
            loaded
        })
        .map_err(|_| IpcError::internal("settings lock poisoned"))
}

/// Saves settings to the app configuration directory.
#[tauri::command]
pub fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, SettingsState>,
    settings: Settings,
) -> Result<(), IpcError> {
    if settings.schema_version != 1 {
        return Err(IpcError {
            code: crate::ipc::ErrorCode::Validation,
            message_key: "errors.invalidSettingsVersion",
            field: Some("schemaVersion"),
            retryable: false,
            detail: "unsupported settings schema".to_owned(),
        });
    }
    let path = settings_path(&app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| IpcError::internal(error.to_string()))?;
    }
    let text = serde_json::to_string_pretty(&settings)
        .map_err(|error| IpcError::internal(error.to_string()))?;
    std::fs::write(path, text).map_err(|error| IpcError::internal(error.to_string()))?;
    crate::logs::info(
        "settings",
        format!(
            "saved: policy={:?} logLevel={} pathLookup={}",
            settings.extractor_policy, settings.log_level, settings.allow_path_lookup
        ),
    );
    state
        .0
        .lock()
        .map(|mut current| *current = settings)
        .map_err(|_| IpcError::internal("settings lock poisoned"))
}
