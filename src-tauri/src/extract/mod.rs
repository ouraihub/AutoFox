//! Parser backend discovery and yt-dlp JSON probing.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{process::Command, time::timeout};
use url::Url;

use crate::{
    ipc::{ErrorCode, IpcError},
    logs,
    system::paths,
};

const TARGET: &str = "extract";
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// Current availability of bundled executables.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    /// Whether the bundled yt-dlp executable exists and responds to `--version`.
    pub ytdlp_ready: bool,
    /// Whether the bundled ffmpeg executable exists.
    pub ffmpeg_ready: bool,
    /// Whether the bundled ffprobe executable exists.
    pub ffprobe_ready: bool,
    /// Detected yt-dlp version.
    pub version: Option<String>,
    /// Path checked for yt-dlp.
    pub ytdlp_path: String,
    /// Path checked for ffmpeg.
    pub ffmpeg_path: String,
}

/// Request to probe one media URL.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRequest {
    /// User-provided HTTP(S) media page URL.
    pub url: String,
}

/// Normalized media metadata returned by every extractor backend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaManifest {
    /// Backend-specific media identifier.
    pub source_id: String,
    /// Display title.
    pub title: String,
    /// Duration in seconds, when supplied by the backend.
    pub duration_secs: Option<f64>,
    /// Available media formats.
    pub formats: Vec<MediaFormat>,
    /// Playlist entries, when playlist probing is enabled.
    pub entries: Vec<PlaylistEntry>,
    /// Backend that produced this manifest.
    pub produced_by: String,
    /// Backend version.
    pub backend_version: String,
}

/// One playlist item that can be probed or queued independently.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    /// Entry identifier.
    pub source_id: String,
    /// Entry title.
    pub title: String,
    /// Entry URL.
    pub url: Option<String>,
    /// Duration in seconds.
    pub duration_secs: Option<f64>,
}

/// One downloadable or streamable media format.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFormat {
    /// Backend-local format identifier.
    pub format_id: String,
    /// Delivery mechanism.
    pub delivery: Delivery,
    /// Container extension.
    pub container: String,
    /// Video properties, if present.
    pub video: Option<VideoSpec>,
    /// Audio properties, if present.
    pub audio: Option<AudioSpec>,
    /// Known file size.
    pub size: SizeHint,
    /// Headers required by the media URL.
    pub http_headers: BTreeMap<String, String>,
}

/// Format delivery mechanism.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Delivery {
    /// A single HTTP resource.
    Direct { url: String },
    /// An HLS playlist.
    Hls { playlist_url: String },
    /// A DASH manifest or representation URL.
    Dash { manifest_url: String },
}

/// Video properties.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSpec {
    /// Video codec name.
    pub codec: String,
    /// Pixel width.
    pub width: Option<u64>,
    /// Pixel height.
    pub height: Option<u64>,
    /// Frames per second.
    pub fps: Option<f64>,
    /// Total bitrate in kilobits per second.
    pub bitrate_kbps: Option<f64>,
}

/// Audio properties.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSpec {
    /// Audio codec name.
    pub codec: String,
    /// Number of channels.
    pub channels: Option<u64>,
    /// Audio sample rate.
    pub sample_rate: Option<u64>,
    /// Bitrate in kilobits per second.
    pub bitrate_kbps: Option<f64>,
}

/// Size information from the extractor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "bytes")]
pub enum SizeHint {
    /// Exact size.
    Exact(u64),
    /// Approximate size.
    Approx(u64),
    /// No size available.
    Unknown,
}

/// Returns the current bundled backend status.
pub async fn backend_status(
    configured_ytdlp: Option<&str>,
    configured_ffmpeg: Option<&str>,
) -> Result<BackendStatus, IpcError> {
    let bundled_ytdlp =
        paths::bundled_ytdlp().map_err(|error| IpcError::internal(error.to_string()))?;
    let bundled_ffmpeg =
        paths::bundled_ffmpeg().map_err(|error| IpcError::internal(error.to_string()))?;
    let ytdlp = paths::configured_or_bundled(configured_ytdlp, bundled_ytdlp);
    let ffmpeg = paths::configured_or_bundled(configured_ffmpeg, bundled_ffmpeg);
    let ffprobe =
        paths::bundled_ffprobe().map_err(|error| IpcError::internal(error.to_string()))?;
    let ytdlp_version = if is_executable_file(&ytdlp) {
        detect_version(&ytdlp).await
    } else {
        None
    };
    let status = BackendStatus {
        ytdlp_ready: ytdlp_version.is_some(),
        ffmpeg_ready: is_executable_file(&ffmpeg),
        ffprobe_ready: is_executable_file(&ffprobe),
        version: ytdlp_version,
        ytdlp_path: display_path(ytdlp),
        ffmpeg_path: display_path(ffmpeg),
    };
    logs::info(
        TARGET,
        format!(
            "backend check: yt-dlp={} ffmpeg={} ffprobe={}",
            status.ytdlp_ready, status.ffmpeg_ready, status.ffprobe_ready
        ),
    );
    Ok(status)
}

/// Reports bundled parser and muxer availability to the frontend.
#[tauri::command]
pub async fn get_backend_status(
    state: tauri::State<'_, crate::settings::SettingsState>,
) -> Result<BackendStatus, IpcError> {
    let settings = state
        .0
        .lock()
        .map_err(|_| IpcError::internal("settings lock poisoned"))?
        .clone();
    backend_status(
        settings.ytdlp_path.as_deref(),
        settings.ffmpeg_path.as_deref(),
    )
    .await
}

/// Parses a URL with the bundled yt-dlp executable.
#[tauri::command]
pub async fn probe_url(
    state: tauri::State<'_, crate::settings::SettingsState>,
    request: ProbeRequest,
) -> Result<MediaManifest, IpcError> {
    let settings = state
        .0
        .lock()
        .map_err(|_| IpcError::internal("settings lock poisoned"))?
        .clone();
    let url = validate_url(&request.url).inspect_err(|_| {
        logs::warn(
            TARGET,
            format!("rejected URL: {}", logs::redact_url(&request.url)),
        );
    })?;
    logs::info(
        TARGET,
        format!("probing {}", logs::redact_url(url.as_str())),
    );
    let bundled = paths::bundled_ytdlp().map_err(|error| IpcError::internal(error.to_string()))?;
    let executable = paths::configured_or_bundled(settings.ytdlp_path.as_deref(), bundled);
    if !is_executable_file(&executable) {
        logs::error(TARGET, "yt-dlp executable not found");
        return Err(IpcError {
            code: ErrorCode::BackendUnavailable,
            message_key: "errors.ytdlpMissing",
            field: Some("url"),
            retryable: false,
            detail: "bundled yt-dlp is not available".to_owned(),
        });
    }
    logs::debug(TARGET, format!("using backend {}", executable.display()));

    let output = timeout(
        PROBE_TIMEOUT,
        Command::new(&executable)
            .args([
                "--dump-single-json",
                "--skip-download",
                "--no-playlist",
                "--no-warnings",
                "--no-progress",
                "--",
                url.as_str(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| IpcError {
        code: ErrorCode::TimedOut,
        message_key: "errors.probeTimedOut",
        field: None,
        retryable: true,
        detail: "media probing timed out".to_owned(),
    })?
    .map_err(|error| IpcError {
        code: ErrorCode::BackendUnavailable,
        message_key: "errors.ytdlpLaunchFailed",
        field: None,
        retryable: false,
        detail: format!("could not start yt-dlp: {error}"),
    })?;

    if !output.status.success() {
        let failure = classify_process_error(&output.stderr);
        logs::error(TARGET, format!("yt-dlp failed: {}", failure.detail));
        return Err(failure);
    }
    let payload: Value = serde_json::from_slice(&output.stdout).map_err(|_| IpcError {
        code: ErrorCode::BackendProtocol,
        message_key: "errors.invalidBackendJson",
        field: None,
        retryable: false,
        detail: "yt-dlp returned invalid JSON".to_owned(),
    })?;
    let version = detect_version(&executable)
        .await
        .unwrap_or_else(|| "unknown".to_owned());
    let manifest = manifest_from_json(&payload, version)?;
    logs::info(
        TARGET,
        format!(
            "parsed {} format(s) via yt-dlp {}",
            manifest.formats.len(),
            manifest.backend_version
        ),
    );
    Ok(manifest)
}

async fn detect_version(executable: &Path) -> Option<String> {
    let output = timeout(
        VERSION_TIMEOUT,
        Command::new(executable).arg("--version").output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!version.is_empty()).then_some(version)
}

fn validate_url(raw: &str) -> Result<Url, IpcError> {
    let parsed = Url::parse(raw).map_err(|_| invalid_url())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err(invalid_url());
    }
    Ok(parsed)
}

fn invalid_url() -> IpcError {
    IpcError {
        code: ErrorCode::Validation,
        message_key: "errors.invalidUrl",
        field: Some("url"),
        retryable: false,
        detail: "only an http(s) URL is accepted".to_owned(),
    }
}

fn classify_process_error(stderr: &[u8]) -> IpcError {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    let (code, message_key, retryable) = if text.contains("unsupported url") {
        (ErrorCode::Unsupported, "errors.unsupportedUrl", false)
    } else if text.contains("private video") || text.contains("sign in") || text.contains("login") {
        (ErrorCode::AuthRequired, "errors.authRequired", false)
    } else if text.contains("not available in your country") || text.contains("geo") {
        (ErrorCode::GeoRestricted, "errors.geoRestricted", false)
    } else if text.contains("timed out") || text.contains("timeout") {
        (ErrorCode::TimedOut, "errors.probeTimedOut", true)
    } else if text.contains("http error") || text.contains("unable to download") {
        (ErrorCode::Network, "errors.network", true)
    } else {
        (
            ErrorCode::ExtractionFailed,
            "errors.extractionFailed",
            false,
        )
    };
    IpcError {
        code,
        message_key,
        field: None,
        retryable,
        detail: "yt-dlp could not probe this URL".to_owned(),
    }
}

fn manifest_from_json(payload: &Value, backend_version: String) -> Result<MediaManifest, IpcError> {
    let source_id = string_field(payload, "id").unwrap_or_else(|| "unknown".to_owned());
    let title = string_field(payload, "title").unwrap_or_else(|| source_id.clone());
    let formats = payload
        .get("formats")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(format_from_json)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if formats.is_empty() {
        return Err(IpcError {
            code: ErrorCode::FormatUnavailable,
            message_key: "errors.noFormats",
            field: None,
            retryable: false,
            detail: "yt-dlp returned no downloadable formats".to_owned(),
        });
    }
    let entries = payload
        .get("entries")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(PlaylistEntry {
                        source_id: string_field(item, "id")?,
                        title: string_field(item, "title").unwrap_or_else(|| "untitled".to_owned()),
                        url: string_field(item, "webpage_url")
                            .or_else(|| string_field(item, "url")),
                        duration_secs: number_field(item, "duration"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(MediaManifest {
        source_id,
        title,
        duration_secs: number_field(payload, "duration"),
        formats,
        entries,
        produced_by: "ytdlp".to_owned(),
        backend_version,
    })
}

fn format_from_json(value: &Value) -> Option<MediaFormat> {
    let url = string_field(value, "url")?;
    let protocol = string_field(value, "protocol").unwrap_or_default();
    let delivery = if protocol.contains("m3u8") {
        Delivery::Hls { playlist_url: url }
    } else if protocol.contains("dash") {
        Delivery::Dash { manifest_url: url }
    } else {
        Delivery::Direct { url }
    };
    let vcodec = string_field(value, "vcodec").filter(|codec| codec != "none");
    let acodec = string_field(value, "acodec").filter(|codec| codec != "none");
    let video = vcodec.map(|codec| VideoSpec {
        codec,
        width: integer_field(value, "width"),
        height: integer_field(value, "height"),
        fps: number_field(value, "fps"),
        bitrate_kbps: number_field(value, "vbr").or_else(|| number_field(value, "tbr")),
    });
    let audio = acodec.map(|codec| AudioSpec {
        codec,
        channels: integer_field(value, "audio_channels"),
        sample_rate: integer_field(value, "asr"),
        bitrate_kbps: number_field(value, "abr").or_else(|| number_field(value, "tbr")),
    });
    let size = integer_field(value, "filesize")
        .map(SizeHint::Exact)
        .or_else(|| integer_field(value, "filesize_approx").map(SizeHint::Approx))
        .unwrap_or(SizeHint::Unknown);
    let http_headers = value
        .get("http_headers")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(MediaFormat {
        format_id: string_field(value, "format_id").unwrap_or_else(|| "unknown".to_owned()),
        delivery,
        container: string_field(value, "ext").unwrap_or_else(|| "unknown".to_owned()),
        video,
        audio,
        size,
        http_headers,
    })
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn number_field(value: &Value, field: &str) -> Option<f64> {
    value.get(field).and_then(Value::as_f64)
}

fn integer_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn display_path(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::{Delivery, SizeHint, manifest_from_json};
    use serde_json::json;

    #[test]
    fn maps_ytdlp_formats_to_normalized_manifest() {
        let payload = json!({
            "id": "abc",
            "title": "Example",
            "duration": 12.5,
            "formats": [{
                "format_id": "18",
                "url": "https://cdn.example/video.mp4",
                "protocol": "https",
                "ext": "mp4",
                "vcodec": "h264",
                "acodec": "aac",
                "width": 640,
                "height": 360,
                "filesize": 1024
            }]
        });
        let manifest = manifest_from_json(&payload, "test".to_owned()).expect("fixture should map");
        assert_eq!(manifest.title, "Example");
        assert_eq!(manifest.formats.len(), 1);
        assert!(matches!(
            manifest.formats[0].delivery,
            Delivery::Direct { .. }
        ));
        assert!(matches!(manifest.formats[0].size, SizeHint::Exact(1024)));
    }
}
