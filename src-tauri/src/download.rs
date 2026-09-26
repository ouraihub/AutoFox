//! Direct HTTP download service with resumable `.part` files.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use crate::system::http::ensure_crypto_provider;
use reqwest::{
    Client, StatusCode,
    header::{HeaderMap, HeaderName, HeaderValue, RANGE},
};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{self, File, OpenOptions},
    io::AsyncWriteExt,
};

use crate::ipc::{ErrorCode, IpcError};
use crate::logs;

const TARGET: &str = "download";
const DEFAULT_FILE_NAME: &str = "autofox-download";
const CHUNK_SIZE: usize = 64 * 1024;

/// User request for one direct media download.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    /// URL of the direct media resource.
    pub url: String,
    /// Backend format identifier, used for the output extension.
    pub format_id: String,
    /// Output directory selected by the user.
    pub output_directory: String,
    /// Output file name without an extension.
    pub file_name: Option<String>,
    /// Optional headers required by the media URL.
    #[serde(default)]
    pub http_headers: std::collections::BTreeMap<String, String>,
}

/// Download result returned after the atomic rename succeeds.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    /// Stable task identifier.
    pub task_id: String,
    /// Final output path.
    pub output_path: String,
    /// Total bytes written.
    pub bytes_written: u64,
}

/// Progress event emitted while a task is downloading.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// Stable task identifier.
    pub task_id: String,
    /// Bytes currently written.
    pub bytes_written: u64,
    /// Expected total bytes, if known.
    pub total_bytes: Option<u64>,
    /// Current bytes per second.
    pub bytes_per_second: u64,
}

/// Downloads a direct HTTP format to an atomic final file.
#[tauri::command]
pub async fn download_direct(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::queue::TaskRegistry>,
    request: DownloadRequest,
) -> Result<DownloadResult, IpcError> {
    let (task_id, cancel) = state.start("direct", Some(&app))?;
    logs::info(
        TARGET,
        format!("task {task_id} started: {}", logs::redact_url(&request.url)),
    );
    let result = download_direct_task(Some(&app), request, task_id.clone(), cancel).await;
    match &result {
        Ok(value) => {
            logs::info(
                TARGET,
                format!("task {task_id} wrote {} byte(s)", value.bytes_written),
            );
            state.finish(
                &task_id,
                crate::queue::TaskState::Done,
                Some(value.output_path.clone()),
                None,
            );
        }
        Err(error) => {
            if error.code == ErrorCode::Cancelled {
                logs::warn(TARGET, format!("task {task_id} cancelled"));
            } else {
                logs::error(TARGET, format!("task {task_id} failed: {}", error.detail));
            }
            state.finish(
                &task_id,
                if error.code == ErrorCode::Cancelled {
                    crate::queue::TaskState::Cancelled
                } else {
                    crate::queue::TaskState::Failed
                },
                None,
                Some(error.message_key.to_owned()),
            );
        }
    }
    result
}

#[cfg(test)]
async fn download_direct_inner(
    app: Option<&tauri::AppHandle>,
    request: DownloadRequest,
) -> Result<DownloadResult, IpcError> {
    let task_id = uuid::Uuid::new_v4().to_string();
    download_direct_task(app, request, task_id, Arc::new(AtomicBool::new(false))).await
}

async fn download_direct_task(
    app: Option<&tauri::AppHandle>,
    request: DownloadRequest,
    task_id: String,
    cancel: Arc<AtomicBool>,
) -> Result<DownloadResult, IpcError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(IpcError {
            code: ErrorCode::Cancelled,
            message_key: "errors.cancelled",
            field: None,
            retryable: false,
            detail: "download cancelled".to_owned(),
        });
    }
    let url =
        url::Url::parse(&request.url).map_err(|_| validation("url", "invalid download URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(validation("url", "only an http(s) URL is accepted"));
    }
    let output_dir = PathBuf::from(&request.output_directory);
    fs::create_dir_all(&output_dir)
        .await
        .map_err(|error| io_error("could not create output directory", error))?;
    let safe_name = sanitize_file_name(request.file_name.as_deref().unwrap_or(DEFAULT_FILE_NAME));
    let extension = extension_for_format(&request.format_id);
    let final_path = output_dir.join(format!("{safe_name}.{extension}"));
    let part_path = output_dir.join(format!("{safe_name}.{extension}.part"));

    ensure_crypto_provider();
    let client = Client::new();
    let mut headers = HeaderMap::new();
    for (name, value) in request.http_headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| validation("httpHeaders", "invalid header name"))?;
        let header_value = HeaderValue::from_str(&value)
            .map_err(|_| validation("httpHeaders", "invalid header value"))?;
        headers.insert(header_name, header_value);
    }

    let existing = file_len(&part_path)
        .await
        .map_err(|error| io_error("could not inspect partial file", error))?;
    if existing > 0 {
        headers.insert(
            RANGE,
            HeaderValue::from_str(&format!("bytes={existing}-"))
                .map_err(|_| IpcError::internal("invalid range header"))?,
        );
    }
    let response = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|error| network_error(error.to_string()))?;
    let append = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() && response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(network_error(format!(
            "download server returned status {}",
            response.status()
        )));
    }
    let total_bytes = response.content_length().map(|size| {
        if append {
            size.saturating_add(existing)
        } else {
            size
        }
    });
    let mut file = if append {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&part_path)
            .await
    } else {
        File::create(&part_path).await
    }
    .map_err(|error| io_error("could not open partial file", error))?;

    let mut stream = response.bytes_stream();
    let mut bytes_written = if append { existing } else { 0 };
    let started = Instant::now();
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        if cancel.load(Ordering::Relaxed) {
            return Err(IpcError {
                code: ErrorCode::Cancelled,
                message_key: "errors.cancelled",
                field: None,
                retryable: false,
                detail: "download cancelled".to_owned(),
            });
        }
        let chunk = chunk.map_err(|error| network_error(error.to_string()))?;
        write_chunk(&mut file, &chunk)
            .await
            .map_err(|error| io_error("could not write partial file", error))?;
        bytes_written = bytes_written.saturating_add(chunk.len() as u64);
        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        let event = DownloadProgress {
            task_id: task_id.clone(),
            bytes_written,
            total_bytes,
            bytes_per_second: (bytes_written as f64 / elapsed) as u64,
        };
        if let Some(app) = app {
            let _ = tauri::Emitter::emit(app, "task://progress", event);
        }
    }
    file.flush()
        .await
        .map_err(|error| io_error("could not flush partial file", error))?;
    drop(file);
    fs::rename(&part_path, &final_path)
        .await
        .map_err(|error| io_error("could not finalize downloaded file", error))?;
    if let Some(app) = app {
        let _ = tauri::Emitter::emit(
            app,
            "task://state",
            serde_json::json!({ "taskId": task_id, "state": "done" }),
        );
    }
    Ok(DownloadResult {
        task_id,
        output_path: final_path.to_string_lossy().into_owned(),
        bytes_written,
    })
}

async fn file_len(path: &Path) -> std::io::Result<u64> {
    match fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

async fn write_chunk(file: &mut File, chunk: &[u8]) -> std::io::Result<()> {
    for part in chunk.chunks(CHUNK_SIZE) {
        file.write_all(part).await?;
    }
    Ok(())
}

fn sanitize_file_name(name: &str) -> String {
    let mut output = name
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect::<String>();
    output = output.trim().trim_matches('.').to_owned();
    if output.is_empty() {
        DEFAULT_FILE_NAME.to_owned()
    } else {
        output.chars().take(180).collect()
    }
}

fn extension_for_format(format_id: &str) -> &'static str {
    if format_id.contains("audio") {
        "m4a"
    } else {
        "mp4"
    }
}

fn validation(field: &'static str, detail: &'static str) -> IpcError {
    IpcError {
        code: ErrorCode::Validation,
        message_key: "errors.invalidDownloadRequest",
        field: Some(field),
        retryable: false,
        detail: detail.to_owned(),
    }
}

fn io_error(detail: &'static str, error: std::io::Error) -> IpcError {
    IpcError {
        code: ErrorCode::AtomicWrite,
        message_key: "errors.fileWriteFailed",
        field: None,
        retryable: true,
        detail: format!("{detail}: {error}"),
    }
}

fn network_error(detail: String) -> IpcError {
    IpcError {
        code: ErrorCode::Network,
        message_key: "errors.downloadFailed",
        field: None,
        retryable: true,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::{extension_for_format, sanitize_file_name};

    #[test]
    fn sanitizes_windows_path_characters() {
        assert_eq!(sanitize_file_name(r#"a<b>:c/d"#), "a_b__c_d");
    }

    #[test]
    fn chooses_safe_default_extension() {
        assert_eq!(extension_for_format("18"), "mp4");
        assert_eq!(extension_for_format("audio-only"), "m4a");
    }
}

#[cfg(test)]
mod loopback_tests {
    use super::{DownloadRequest, download_direct_inner};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn downloads_direct_resource_and_removes_part_file() {
        let payload = b"autofox direct download smoke".to_vec();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let address = listener.local_addr().expect("read loopback address");
        let server_payload = payload.clone();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut request = [0_u8; 1024];
                let _ = socket.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    server_payload.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&server_payload).await;
            }
        });
        let output =
            std::env::temp_dir().join(format!("autofox-download-test-{}", uuid::Uuid::new_v4()));
        let result = download_direct_inner(
            None,
            DownloadRequest {
                url: format!("http://{address}/media"),
                format_id: "test".to_owned(),
                output_directory: output.to_string_lossy().into_owned(),
                file_name: Some("smoke".to_owned()),
                http_headers: Default::default(),
            },
        )
        .await
        .expect("download should complete");
        assert_eq!(
            tokio::fs::read(&result.output_path)
                .await
                .expect("read output"),
            payload
        );
        assert!(!std::path::PathBuf::from(format!("{}.part", result.output_path)).exists());
        let _ = tokio::fs::remove_dir_all(output).await;
    }
}
