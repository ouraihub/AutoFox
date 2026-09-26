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
