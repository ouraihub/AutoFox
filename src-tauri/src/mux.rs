//! ffmpeg stream copy orchestration for separate audio/video formats.

use std::{path::PathBuf, process::Stdio};

use serde::{Deserialize, Serialize};
use tokio::{
    fs,
    process::Command,
    time::{Duration, timeout},
};

use crate::{
    ipc::{ErrorCode, IpcError},
    logs,
    system::paths,
};

const TARGET: &str = "mux";
const MUX_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Request to mux one video and one audio URL with ffmpeg stream copy.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MuxRequest {
    /// Video stream URL.
    pub video_url: String,
    /// Audio stream URL.
    pub audio_url: String,
    /// Final output directory.
    pub output_directory: String,
    /// Output file name without extension.
    pub file_name: String,
    /// Headers passed to both input streams.
    #[serde(default)]
    pub http_headers: std::collections::BTreeMap<String, String>,
}

/// Result of a successful ffmpeg mux operation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MuxResult {
    /// Final output path.
    pub output_path: String,
}

/// Runs ffmpeg on one HLS or DASH input and atomically writes an MP4 output.
#[tauri::command]
pub async fn download_stream(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::queue::TaskRegistry>,
    request: StreamRequest,
) -> Result<MuxResult, IpcError> {
    let (task_id, _cancel) = state.start("stream", Some(&app))?;
    logs::info(
        TARGET,
        format!("stream task {task_id}: {}", logs::redact_url(&request.url)),
    );
    let result = download_stream_inner(Some(&app), request).await;
    match &result {
        Ok(value) => {
            logs::info(TARGET, format!("stream task {task_id} finished"));
            state.finish(
                &task_id,
                crate::queue::TaskState::Done,
                Some(value.output_path.clone()),
                None,
            );
        }
        Err(error) => {
            logs::error(
                TARGET,
                format!("stream task {task_id} failed: {}", error.detail),
            );
            state.finish(
                &task_id,
                crate::queue::TaskState::Failed,
                None,
                Some(error.message_key.to_owned()),
            );
        }
    }
    result
}

/// One manifest stream passed to ffmpeg.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamRequest {
    /// HLS or DASH URL.
    pub url: String,
    /// Final output directory.
    pub output_directory: String,
    /// Output file name without extension.
    pub file_name: String,
}

async fn download_stream_inner(
    app: Option<&tauri::AppHandle>,
    request: StreamRequest,
) -> Result<MuxResult, IpcError> {
    let input = validate_http_url(&request.url, "url")?;
    let ffmpeg = paths::bundled_ffmpeg().map_err(|error| internal(error.to_string()))?;
    if !ffmpeg.is_file() {
        return Err(IpcError {
            code: ErrorCode::MuxerMissing,
            message_key: "errors.ffmpegMissing",
            field: None,
            retryable: false,
            detail: "bundled ffmpeg is not available".to_owned(),
        });
    }
    let output_dir = PathBuf::from(&request.output_directory);
    fs::create_dir_all(&output_dir)
        .await
        .map_err(|error| io_error(error.to_string()))?;
    let name = sanitize_file_name(&request.file_name);
    let final_path = output_dir.join(format!("{name}.mp4"));
    let temp_path = output_dir.join(format!("{name}.mp4.part"));
    let mut command = Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-y",
        "-i",
        input.as_str(),
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        "-progress",
        "pipe:1",
        "-nostats",
        temp_path.to_string_lossy().as_ref(),
    ]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = timeout(MUX_TIMEOUT, command.output())
        .await
        .map_err(|_| timed_out())?
        .map_err(|error| internal(error.to_string()))?;
    emit_ffmpeg_progress(app, &output.stdout);
    if !output.status.success() {
        return Err(IpcError {
            code: ErrorCode::MuxFailed,
            message_key: "errors.muxFailed",
            field: None,
            retryable: false,
            detail: "ffmpeg could not download the stream".to_owned(),
        });
    }
    fs::rename(&temp_path, &final_path)
        .await
        .map_err(|error| io_error(error.to_string()))?;
    Ok(MuxResult {
        output_path: final_path.to_string_lossy().into_owned(),
    })
}

fn emit_ffmpeg_progress(app: Option<&tauri::AppHandle>, stdout: &[u8]) {
    let mut out_time_ms = None;
    let mut speed = None;
    for line in String::from_utf8_lossy(stdout).lines() {
        if let Some(value) = line.strip_prefix("out_time_ms=") {
            out_time_ms = value.parse::<u64>().ok();
        }
        if let Some(value) = line.strip_prefix("speed=") {
            speed = Some(value.to_owned());
        }
    }
    if let Some(app) = app {
        let _ = tauri::Emitter::emit(
            app,
            "task://progress",
            serde_json::json!({ "stage": "muxing", "outTimeMs": out_time_ms, "speed": speed }),
        );
    }
}

/// Runs ffmpeg with separate video and audio inputs and `-c copy`.
#[tauri::command]
pub async fn mux_streams(
    app: tauri::AppHandle,
    request: MuxRequest,
) -> Result<MuxResult, IpcError> {
    logs::info(
        TARGET,
        format!(
            "muxing video {} with audio {}",
            logs::redact_url(&request.video_url),
            logs::redact_url(&request.audio_url)
        ),
    );
    mux_streams_inner(Some(&app), request)
        .await
        .inspect(|_| logs::info(TARGET, "mux finished"))
        .inspect_err(|error| logs::error(TARGET, format!("mux failed: {}", error.detail)))
}

async fn mux_streams_inner(
    app: Option<&tauri::AppHandle>,
    request: MuxRequest,
) -> Result<MuxResult, IpcError> {
    let video = validate_http_url(&request.video_url, "videoUrl")?;
    let audio = validate_http_url(&request.audio_url, "audioUrl")?;
    let ffmpeg = paths::bundled_ffmpeg().map_err(|error| internal(error.to_string()))?;
    if !ffmpeg.is_file() {
        return Err(IpcError {
            code: ErrorCode::MuxerMissing,
            message_key: "errors.ffmpegMissing",
            field: None,
            retryable: false,
            detail: "bundled ffmpeg is not available".to_owned(),
        });
    }
    let output_dir = PathBuf::from(&request.output_directory);
    fs::create_dir_all(&output_dir)
        .await
        .map_err(|error| io_error(error.to_string()))?;
    let name = sanitize_file_name(&request.file_name);
    let final_path = output_dir.join(format!("{name}.mp4"));
    let temp_path = output_dir.join(format!("{name}.mp4.part"));
    let mut command = Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-y",
        "-i",
        video.as_str(),
        "-i",
        audio.as_str(),
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        "-progress",
        "pipe:1",
        "-nostats",
        temp_path.to_string_lossy().as_ref(),
    ]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = timeout(MUX_TIMEOUT, command.output())
        .await
        .map_err(|_| timed_out())?
        .map_err(|error| internal(error.to_string()))?;
    if !output.status.success() {
        return Err(IpcError {
            code: ErrorCode::MuxFailed,
            message_key: "errors.muxFailed",
            field: None,
            retryable: false,
            detail: "ffmpeg could not combine the selected streams".to_owned(),
        });
    }
    fs::rename(&temp_path, &final_path)
        .await
        .map_err(|error| io_error(error.to_string()))?;
    if let Some(app) = app {
        let _ = tauri::Emitter::emit(
            app,
            "task://state",
            serde_json::json!({ "state": "done", "outputPath": final_path }),
        );
    }
    Ok(MuxResult {
        output_path: final_path.to_string_lossy().into_owned(),
    })
}

fn validate_http_url(value: &str, field: &'static str) -> Result<url::Url, IpcError> {
    let url = url::Url::parse(value).map_err(|_| validation(field))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(validation(field));
    }
    Ok(url)
}

fn sanitize_file_name(value: &str) -> String {
    let name = value
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_owned();
    if name.is_empty() {
        "autofox-mux".to_owned()
    } else {
        name.chars().take(180).collect()
    }
}

fn validation(field: &'static str) -> IpcError {
    IpcError {
        code: ErrorCode::Validation,
        message_key: "errors.invalidMuxRequest",
        field: Some(field),
        retryable: false,
        detail: "only http(s) stream URLs are accepted".to_owned(),
    }
}
fn internal(detail: String) -> IpcError {
    IpcError {
        code: ErrorCode::Internal,
        message_key: "errors.internal",
        field: None,
        retryable: false,
        detail,
    }
}
fn io_error(detail: String) -> IpcError {
    IpcError {
        code: ErrorCode::AtomicWrite,
        message_key: "errors.fileWriteFailed",
        field: None,
        retryable: true,
        detail,
    }
}
fn timed_out() -> IpcError {
    IpcError {
        code: ErrorCode::TimedOut,
        message_key: "errors.muxTimedOut",
        field: None,
        retryable: true,
        detail: "ffmpeg muxing timed out".to_owned(),
    }
}
