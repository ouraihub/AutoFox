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
}
