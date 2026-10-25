//! ffmpeg stream copy orchestration for separate audio/video formats.

#[cfg(test)]
#[path = "mux_tests.rs"]
mod mux_tests;

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
};

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
    /// Target container, e.g. `mp4` or `webm`.
    ///
    /// Sent by the caller because the option list already decided it: a VP9 or
    /// AV1 pair is offered as WebM, and forcing those into MP4 produces a file
    /// whose extension lies about its playability.
    pub container: String,
    /// Headers passed to both input streams.
    #[serde(default)]
    pub http_headers: std::collections::BTreeMap<String, String>,
    /// Media duration from the manifest, used to turn ffmpeg's elapsed media
    /// time into a completion fraction. Absent means an indeterminate bar.
    #[serde(default)]
    pub duration_secs: Option<f64>,
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
    let (task_id, cancel) = state.start("stream", Some(&app))?;
    logs::info(
        TARGET,
        format!("stream task {task_id}: {}", logs::redact_url(&request.url)),
    );
    let result = download_stream_inner(Some(&app), &task_id, request, &cancel).await;
    record_outcome(&state, &task_id, &result);
    result
}

/// Moves a stream/mux task terminal: user cancel is `Cancelled` and a warn,
/// anything else `Failed` and an error, same mapping as `download.rs`.
fn record_outcome(
    state: &crate::queue::TaskRegistry,
    task_id: &str,
    result: &Result<MuxResult, IpcError>,
) {
    use crate::queue::TaskState;
    match result {
        Ok(value) => {
            logs::info(TARGET, format!("task {task_id} finished"));
            state.finish(
                task_id,
                TaskState::Done,
                Some(value.output_path.clone()),
                None,
            );
        }
        Err(error) => {
            let terminal = if error.code == ErrorCode::Cancelled {
                logs::warn(TARGET, format!("task {task_id} cancelled"));
                TaskState::Cancelled
            } else {
                logs::error(TARGET, format!("task {task_id} failed: {}", error.detail));
                TaskState::Failed
            };
            state.finish(task_id, terminal, None, Some(error.message_key.to_owned()));
        }
    }
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
    /// Media duration from the manifest, used to turn ffmpeg's elapsed media
    /// time into a completion fraction. Absent means an indeterminate bar.
    #[serde(default)]
    pub duration_secs: Option<f64>,
}

async fn download_stream_inner(
    app: Option<&tauri::AppHandle>,
    task_id: &str,
    request: StreamRequest,
    cancel: &AtomicBool,
) -> Result<MuxResult, IpcError> {
    let input = validate_http_url(&request.url, "url")?;
    let duration_secs = request.duration_secs;
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
        // See `mux_streams_inner`: ffmpeg cannot infer a container from the
        // `.part` suffix, so the format is named explicitly.
        "-f",
        "mp4",
        "-progress",
        "pipe:1",
        // ffmpeg's default progress cadence follows encoder output, which for a
        // stream copy can mean a single report for the whole job. Pin it to a
        // wall-clock interval so the bar actually moves.
        "-stats_period",
        "0.5",
        "-nostats",
        temp_path.to_string_lossy().as_ref(),
    ]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = run_with_progress(command, task_id, duration_secs, cancel, |event| {
        if let Some(app) = app {
            let _ = tauri::Emitter::emit(app, "task://progress", event);
        }
    })
    .await
    .inspect_err(|_| discard_partial(&temp_path))?;
    if !output.status.success() {
        discard_partial(&temp_path);
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
        .map_err(|error| io_error(error.to_string()))
        .inspect_err(|_| discard_partial(&temp_path))?;
    Ok(MuxResult {
        output_path: final_path.to_string_lossy().into_owned(),
    })
}

/// Runs ffmpeg to completion, emitting `task://progress` as its output grows.
///
/// ffmpeg's `-progress pipe:1` stream is the only progress signal available for
/// a mux: the HTTP reads happen inside the child. Waiting for the process with
/// `Command::output` would buffer that stream and deliver every line after the
/// file is already written, which is no progress at all, so stdout is consumed
/// line by line while the child runs.
///
/// `duration_secs` converts `out_time_ms` into a fraction. Without it only the
/// written byte count is reported, and the bar stays indeterminate.
///
/// Progress goes to `sink` rather than straight to the app handle so the
/// streaming behaviour is observable without a Tauri runtime.
async fn run_with_progress(
    mut command: Command,
    task_id: &str,
    duration_secs: Option<f64>,
    cancel: &AtomicBool,
    mut sink: impl FnMut(crate::download::DownloadProgress),
) -> Result<std::process::Output, IpcError> {
    // Every early return below (timeout, pipe error, cancel) drops `child`;
    // without this the ffmpeg process outlives the task and keeps writing.
    let mut child = command
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| internal(error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| internal("ffmpeg stdout was not piped".to_owned()))?;
    // stderr must drain concurrently: ffmpeg blocks once its pipe buffer fills,
    // which would deadlock against this stdout loop.
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| internal("ffmpeg stderr was not piped".to_owned()))?;
    let stderr_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        let mut reader = stderr;
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut buffer).await;
        buffer
    });

    let started = std::time::Instant::now();
    let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(stdout));
    let mut total_size = 0_u64;
    let mut elapsed_media: Option<f64> = None;
    let pump = async {
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|error| internal(error.to_string()))?
        {
            if let Some(value) = line.strip_prefix("total_size=") {
                total_size = value.trim().parse().unwrap_or(total_size);
            }
            if let Some(value) = line.strip_prefix("out_time_ms=") {
                // ffmpeg writes `N/A` until the first frame lands, and the key
                // is named `_ms` but carries microseconds.
                elapsed_media = value.trim().parse::<u64>().ok().map(|us| us as f64 / 1e6);
            }
            // `progress=` terminates each block. Keying on it rather than on a
            // parsed time keeps the early `N/A` blocks: they still carry bytes
            // written, and dropping them left the bar frozen for the first
            // second of every mux.
            if !line.starts_with("progress=") {
                continue;
            }
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let event = crate::download::DownloadProgress {
                task_id: task_id.to_owned(),
                bytes_written: total_size,
                total_bytes: None,
                bytes_per_second: (total_size as f64 / elapsed) as u64,
                // No percent until both the duration and a real `out_time_ms`
                // exist; the byte counter carries the UI until then.
                percent: elapsed_media
                    .zip(duration_secs.filter(|total| *total > 0.0))
                    .map(|(media, total)| (media / total).clamp(0.0, 1.0)),
            };
            sink(event);
        }
        Ok::<(), IpcError>(())
    };

    // The pump only wakes on ffmpeg output, so cancellation cannot be checked
    // inside the loop alone: a stalled CDN read would ignore the flag until the
    // 30-minute timeout. Polling in parallel bounds the reaction to 200ms, and
    // the watch races the whole run, including `wait` after the pipes close.
    let watch = async {
        while !cancel.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let run = async {
        pump.await?;
        child
            .wait()
            .await
            .map_err(|error| internal(error.to_string()))
    };
    let finished = tokio::select! {
        result = timeout(MUX_TIMEOUT, run) => result.unwrap_or_else(|_| Err(timed_out())),
        () = watch => Err(cancelled()),
    };
    // A cancel that lands between exit and this check would otherwise let the
    // caller rename the output while the registry already says `Cancelled`.
    let finished = finished.and_then(|status| {
        if cancel.load(Ordering::Relaxed) {
            Err(cancelled())
        } else {
            Ok(status)
        }
    });
    let status = match finished {
        Ok(status) => status,
        Err(error) => {
            // ffmpeg ignores a closed stdout, so it must be killed on every
            // early exit. Awaiting the kill also reaps it, which releases the
            // Windows file lock before the caller deletes the `.part`.
            let _ = child.kill().await;
            return Err(error);
        }
    };
    let stderr = stderr_task.await.unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout: Vec::new(),
        stderr,
    })
}

/// Runs ffmpeg with separate video and audio inputs and `-c copy`.
#[tauri::command]
pub async fn mux_streams(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::queue::TaskRegistry>,
    request: MuxRequest,
) -> Result<MuxResult, IpcError> {
    let (task_id, cancel) = state.start("mux", Some(&app))?;
    logs::info(
        TARGET,
        format!(
            "mux task {task_id}: video {} with audio {}",
            logs::redact_url(&request.video_url),
            logs::redact_url(&request.audio_url)
        ),
    );
    let result = mux_streams_inner(Some(&app), &task_id, request, &cancel, |event| {
        let _ = tauri::Emitter::emit(&app, "task://progress", event);
    })
    .await;
    record_outcome(&state, &task_id, &result);
    result
}

async fn mux_streams_inner(
    app: Option<&tauri::AppHandle>,
    task_id: &str,
    request: MuxRequest,
    cancel: &AtomicBool,
    sink: impl FnMut(crate::download::DownloadProgress),
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
    // An unknown container would reach ffmpeg as `-f <junk>`; reject it here so
    // the error names the real problem instead of surfacing ffmpeg's usage text.
    let container = match request.container.as_str() {
        "mp4" => "mp4",
        "webm" => "webm",
        other => {
            return Err(IpcError {
                code: ErrorCode::Validation,
                message_key: "errors.unsupportedContainer",
                field: Some("container"),
                retryable: false,
                detail: format!("unsupported output container: {other}"),
            });
        }
    };
    let final_path = output_dir.join(format!("{name}.{container}"));
    let temp_path = output_dir.join(format!("{name}.{container}.part"));
    // googlevideo rejects the CDN URLs with 403 unless the request repeats the
    // User-Agent the extractor used, so `-headers` must precede each `-i`: in
    // ffmpeg it is a per-input option, not a global one.
    let header_block = header_block(&request.http_headers);
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-y"]);
    for input in [video.as_str(), audio.as_str()] {
        if let Some(headers) = header_block.as_deref() {
            command.args(["-headers", headers]);
        }
        command.args(["-i", input]);
    }
    command.args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"]);
    // `+faststart` moves the MP4 moov atom to the front for progressive
    // playback. It is an MP4 muxer option; ffmpeg errors out on WebM.
    if container == "mp4" {
        command.args(["-movflags", "+faststart"]);
    }
    command.args([
        // The output is written to `.<ext>.part` so a crash cannot leave a
        // half-written media file behind, but ffmpeg picks the container from
        // the extension and does not know `.part`. Without this it fails for
        // every input with "Unable to choose an output format".
        "-f",
        container,
        "-progress",
        "pipe:1",
        // See `download_stream_inner`: the default cadence can report once for
        // an entire stream copy.
        "-stats_period",
        "0.5",
        "-nostats",
        temp_path.to_string_lossy().as_ref(),
    ]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = run_with_progress(command, task_id, request.duration_secs, cancel, sink)
        .await
        .inspect_err(|_| discard_partial(&temp_path))?;
    if !output.status.success() {
        discard_partial(&temp_path);
        // ffmpeg's own diagnosis is the only way to tell a dead CDN URL from a
        // codec the container rejects, and it never contains the signed query.
        logs::error(
            TARGET,
            format!(
                "ffmpeg mux failed: {}",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .filter(|line| line.contains("Error") || line.contains("Invalid"))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        );
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
        .map_err(|error| io_error(error.to_string()))
        .inspect_err(|_| discard_partial(&temp_path))?;
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

/// Renders headers as the CRLF-delimited block ffmpeg's `-headers` expects.
///
/// A header value carrying its own CRLF would let a caller append arbitrary
/// extra headers to the CDN request, so such entries are dropped rather than
/// escaped: no legitimate value in a yt-dlp manifest contains one.
fn header_block(headers: &std::collections::BTreeMap<String, String>) -> Option<String> {
    let mut block = String::new();
    for (name, value) in headers {
        if name.is_empty() || name.contains([':', '\r', '\n']) || value.contains(['\r', '\n']) {
            continue;
        }
        block.push_str(name);
        block.push_str(": ");
        block.push_str(value);
        block.push_str("\r\n");
    }
    (!block.is_empty()).then_some(block)
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

/// Removes the `.part` file a failed or cancelled ffmpeg left behind.
///
/// A `-c copy` output cut mid-write has no trailing index, so it is neither
/// playable nor resumable, and nothing reads it after the task ends.
fn discard_partial(temp_path: &Path) {
    match std::fs::remove_file(temp_path) {
        Ok(()) => logs::info(TARGET, "discarded incomplete output"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => logs::warn(TARGET, format!("cannot remove incomplete output: {error}")),
    }
}
/// Matches `download.rs`: cancellation is a distinct code so the queue can move
/// the task to `Cancelled` instead of `Failed`.
fn cancelled() -> IpcError {
    IpcError {
        code: ErrorCode::Cancelled,
        message_key: "errors.cancelled",
        field: None,
        retryable: false,
        detail: "mux cancelled".to_owned(),
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
