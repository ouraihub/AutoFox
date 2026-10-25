//! Stable IPC error envelope shared by AutoFox commands.

use serde::Serialize;

/// Machine-readable error codes exposed to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum ErrorCode {
    /// Input validation failed.
    Validation,
    /// No parser backend is ready.
    NoBackendAvailable,
    /// A configured backend executable is unavailable.
    BackendUnavailable,
    /// A backend did not satisfy its protocol.
    BackendProtocol,
    /// The selected backend does not support the URL.
    Unsupported,
    /// Extraction failed.
    ExtractionFailed,
    /// Authentication is required.
    AuthRequired,
    /// The media is geographically restricted.
    GeoRestricted,
    /// The selected format is no longer available.
    FormatUnavailable,
    /// A network request failed.
    Network,
    /// An operation timed out.
    TimedOut,
    /// The user cancelled an operation.
    Cancelled,
    /// The destination disk is full.
    DiskFull,
    /// An atomic file operation failed.
    AtomicWrite,
    /// Media muxing failed.
    MuxFailed,
    /// The muxer executable is missing.
    MuxerMissing,
    /// The requested task or resource was not found.
    NotFound,
    /// A concurrency limit is already saturated.
    Busy,
    /// An unexpected internal error occurred.
    Internal,
}

/// Stable command rejection envelope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    /// Machine-readable error code.
    pub code: ErrorCode,
    /// Frontend translation key.
    pub message_key: &'static str,
    /// Optional field associated with the error.
    pub field: Option<&'static str>,
    /// Whether retrying may succeed.
    pub retryable: bool,
    /// Safe human-readable detail without URLs or secrets.
    pub detail: String,
}

impl IpcError {
    /// Creates a safe internal error.
    pub fn internal(detail: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::Internal,
            message_key: "errors.internal",
            field: None,
            retryable: false,
            detail: detail.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorCode;

    /// The TypeScript client hand-maintains a union mirroring [`ErrorCode`].
    /// Nothing at either compiler's disposal links the two, so a variant added
    /// here silently reaches the frontend as an unhandled string. `NotFound`
    /// already drifted that way. The `match` below is exhaustive on purpose:
    /// adding a variant fails to compile until it is listed, and the listed
    /// name is then required to appear in the union.
    #[test]
    fn typescript_union_covers_every_variant() {
        const CLIENT_TS: &str = include_str!("../../src/lib/ipc/client.ts");

        let all = [
            ErrorCode::Validation,
            ErrorCode::NoBackendAvailable,
            ErrorCode::BackendUnavailable,
            ErrorCode::BackendProtocol,
            ErrorCode::Unsupported,
            ErrorCode::ExtractionFailed,
            ErrorCode::AuthRequired,
            ErrorCode::GeoRestricted,
            ErrorCode::FormatUnavailable,
            ErrorCode::Network,
            ErrorCode::TimedOut,
            ErrorCode::Cancelled,
            ErrorCode::DiskFull,
            ErrorCode::AtomicWrite,
            ErrorCode::MuxFailed,
            ErrorCode::MuxerMissing,
            ErrorCode::NotFound,
            ErrorCode::Busy,
            ErrorCode::Internal,
        ];

        let union = CLIENT_TS
            .split_once("export type ErrorCode =")
            .expect("client.ts declares an ErrorCode union")
            .1
            .split_once(';')
            .expect("the union declaration is terminated")
            .0;

        for code in all {
            // Exhaustiveness: a new variant must be added to `all` to compile.
            let name = match code {
                ErrorCode::Validation => "Validation",
                ErrorCode::NoBackendAvailable => "NoBackendAvailable",
                ErrorCode::BackendUnavailable => "BackendUnavailable",
                ErrorCode::BackendProtocol => "BackendProtocol",
                ErrorCode::Unsupported => "Unsupported",
                ErrorCode::ExtractionFailed => "ExtractionFailed",
                ErrorCode::AuthRequired => "AuthRequired",
                ErrorCode::GeoRestricted => "GeoRestricted",
                ErrorCode::FormatUnavailable => "FormatUnavailable",
                ErrorCode::Network => "Network",
                ErrorCode::TimedOut => "TimedOut",
                ErrorCode::Cancelled => "Cancelled",
                ErrorCode::DiskFull => "DiskFull",
                ErrorCode::AtomicWrite => "AtomicWrite",
                ErrorCode::MuxFailed => "MuxFailed",
                ErrorCode::MuxerMissing => "MuxerMissing",
                ErrorCode::NotFound => "NotFound",
                ErrorCode::Busy => "Busy",
                ErrorCode::Internal => "Internal",
            };
            assert!(
                union.contains(&format!("'{name}'")),
                "client.ts ErrorCode union is missing {name}"
            );
        }
    }

    /// Serializes the real producer types into the fixture the TypeScript
    /// parser tests consume, so `contract.test.ts` checks the parsers against
    /// what serde actually emits rather than against hand-written payloads.
    /// Each type appears twice: every optional field set, then every optional
    /// field `None` with collections empty, the shapes most likely to drift.
    ///
    /// Fails when the checked-in file is stale. Regenerate with
    /// `AUTOFOX_UPDATE_FIXTURES=1 pnpm run test:rust`, then run `pnpm run test`.
    #[test]
    fn ipc_fixture_matches_rust_serialization() {
        use crate::{
            download::{DownloadProgress, DownloadResult},
            extract::{
                AudioSpec, BackendStatus, Delivery, MediaFormat, MediaManifest, PlaylistEntry,
                SizeHint, VideoSpec,
                options::{DeliveryPlan, DownloadOption},
            },
            logs::LogEntry,
            mux::MuxResult,
            queue::{TaskState, TaskSummary},
            settings::{ExtractorPolicy, Settings},
        };
        use serde_json::{json, to_value};

        let full_format = MediaFormat {
            format_id: "137".to_owned(),
            delivery: Delivery::Direct {
                url: "https://cdn.example/v.mp4".to_owned(),
            },
            container: "mp4".to_owned(),
            video: Some(VideoSpec {
                codec: "avc1".to_owned(),
                width: Some(1920),
                height: Some(1080),
                fps: Some(30.0),
                bitrate_kbps: Some(4400.5),
            }),
            audio: Some(AudioSpec {
                codec: "mp4a".to_owned(),
                channels: Some(2),
                sample_rate: Some(44100),
                bitrate_kbps: Some(128.0),
            }),
            size: SizeHint::Exact(1024),
            http_headers: [("User-Agent".to_owned(), "AutoFox".to_owned())].into(),
        };
        let bare_format = |delivery| MediaFormat {
            format_id: "hls-1".to_owned(),
            delivery,
            container: "mp4".to_owned(),
            video: None,
            audio: None,
            size: SizeHint::Unknown,
            http_headers: Default::default(),
        };
        let option = |plan, height| DownloadOption {
            option_id: "o1".to_owned(),
            label: "1080p".to_owned(),
            container: "mp4".to_owned(),
            detail: "avc1 + mp4a".to_owned(),
            size: SizeHint::Approx(2048),
            height,
            plan,
        };
        let manifest_full = MediaManifest {
            source_id: "abc".to_owned(),
            title: "Example".to_owned(),
            duration_secs: Some(12.5),
            formats: vec![
                full_format,
                bare_format(Delivery::Hls {
                    playlist_url: "https://cdn.example/p.m3u8".to_owned(),
                }),
                bare_format(Delivery::Dash {
                    manifest_url: "https://cdn.example/m.mpd".to_owned(),
                }),
            ],
            download_options: vec![
                option(
                    DeliveryPlan::Progressive {
                        format_id: "18".to_owned(),
                        url: "https://cdn.example/v.mp4".to_owned(),
                    },
                    Some(360),
                ),
                option(
                    DeliveryPlan::Merge {
                        video_url: "https://cdn.example/v".to_owned(),
                        audio_url: "https://cdn.example/a".to_owned(),
                        video_format_id: "137".to_owned(),
                        audio_format_id: "140".to_owned(),
                    },
                    Some(1080),
                ),
                option(
                    DeliveryPlan::Manifest {
                        format_id: "hls-1".to_owned(),
                        url: "https://cdn.example/p.m3u8".to_owned(),
                    },
                    None,
                ),
            ],
            entries: vec![PlaylistEntry {
                source_id: "e1".to_owned(),
                title: "Entry".to_owned(),
                url: Some("https://example.com/e1".to_owned()),
                duration_secs: Some(3.0),
            }],
            produced_by: "builtin-ytdlp".to_owned(),
            backend_version: "2026.08.19".to_owned(),
        };
        let manifest_bare = MediaManifest {
            source_id: "abc".to_owned(),
            title: "Example".to_owned(),
            duration_secs: None,
            formats: Vec::new(),
            download_options: Vec::new(),
            entries: Vec::new(),
            produced_by: "builtin-ytdlp".to_owned(),
            backend_version: "2026.08.19".to_owned(),
        };
        let backend = |version: Option<&str>| BackendStatus {
            ytdlp_ready: true,
            ffmpeg_ready: version.is_some(),
            ffprobe_ready: false,
            version: version.map(str::to_owned),
            ytdlp_path: "D:\\AutoFox\\yt-dlp.exe".to_owned(),
            ffmpeg_path: "D:\\AutoFox\\lib\\ffmpeg.exe".to_owned(),
        };
        let settings_full = Settings {
            download_directory: Some("D:\\Downloads".to_owned()),
            extractor_policy: ExtractorPolicy::YtdlpOnly,
            ytdlp_path: Some("D:\\tools\\yt-dlp.exe".to_owned()),
            ffmpeg_path: Some("D:\\tools\\ffmpeg.exe".to_owned()),
            ..Settings::default()
        };
        let task = |state, output: Option<&str>, error: Option<&str>| TaskSummary {
            task_id: "t1".to_owned(),
            kind: "mux".to_owned(),
            state,
            output_path: output.map(str::to_owned),
            error_key: error.map(str::to_owned),
        };
        let progress = |total, percent| DownloadProgress {
            task_id: "t1".to_owned(),
            bytes_written: 512,
            total_bytes: total,
            bytes_per_second: 64,
            percent,
        };
        let ser = |value: serde_json::Result<serde_json::Value>| value.expect("serialize");

        let fixture = json!({
            "manifest": [ser(to_value(&manifest_full)), ser(to_value(&manifest_bare))],
            "backendStatus": [ser(to_value(backend(Some("2026.08.19")))), ser(to_value(backend(None)))],
            "settings": [ser(to_value(&settings_full)), ser(to_value(Settings::default()))],
            "tasks": [ser(to_value(vec![
                task(TaskState::Done, Some("D:\\out\\a.mp4"), None),
                task(TaskState::Failed, None, Some("errors.muxFailed")),
            ]))],
            "progress": [ser(to_value(progress(Some(1024), Some(0.5)))), ser(to_value(progress(None, None)))],
            "downloadResult": [ser(to_value(DownloadResult {
                task_id: "t1".to_owned(),
                output_path: "D:\\out\\a.mp4".to_owned(),
                bytes_written: 1024,
            }))],
            "muxResult": [ser(to_value(MuxResult { output_path: "D:\\out\\a.mp4".to_owned() }))],
            "logs": [ser(to_value(vec![LogEntry {
                seq: 1,
                time: "12:34:56.789".to_owned(),
                level: "INFO".to_owned(),
                target: "mux".to_owned(),
                message: "task t1 finished".to_owned(),
            }]))],
        });
        let rendered = format!(
            "{}\n",
            serde_json::to_string_pretty(&fixture).expect("render")
        );

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../src/lib/ipc/fixtures/ipc-payloads.json");
        if std::env::var_os("AUTOFOX_UPDATE_FIXTURES").is_some() {
            std::fs::write(&path, &rendered).expect("write fixture");
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            committed == rendered,
            "src/lib/ipc/fixtures/ipc-payloads.json is stale; run \
             `AUTOFOX_UPDATE_FIXTURES=1 pnpm run test:rust` and commit the result"
        );
    }
}
