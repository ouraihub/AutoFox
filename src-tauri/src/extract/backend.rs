//! Extractor backend identity and the fallback boundary from
//! `docs/03-后端契约.md` §1 and §5.
//!
//! The download engine never learns which backend produced a `MediaManifest`.
//! That is what lets a native Rust YouTube extractor land beside the bundled
//! yt-dlp process without touching the download path.

use std::fmt;

use crate::ipc::ErrorCode;

/// Stable identity of one extractor backend.
///
/// Borrowed rather than owned: every backend is a long-lived singleton, so an
/// id never needs to outlive the registry that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendId(pub &'static str);

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Id of the bundled yt-dlp process backend.
pub const PROCESS_YTDLP: BackendId = BackendId("builtin-ytdlp");

/// Whether a backend error may be retried on the next candidate.
///
/// §5 of the contract: only `unsupported` and `extractionFailed` fall through.
/// Falling through on `network` or `timedOut` would serialize one timeout per
/// backend on an offline machine and report "everything failed" instead of the
/// real cause.
pub fn triggers_fallback(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::Unsupported | ErrorCode::ExtractionFailed)
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, triggers_fallback};

    /// The boundary the contract calls the easiest thing to get wrong: an
    /// offline machine must fail once with `network`, not once per backend.
    #[test]
    fn only_unsupported_and_extraction_failed_fall_through() {
        assert!(triggers_fallback(ErrorCode::Unsupported));
        assert!(triggers_fallback(ErrorCode::ExtractionFailed));
        for code in [
            ErrorCode::Network,
            ErrorCode::TimedOut,
            ErrorCode::Cancelled,
            ErrorCode::AuthRequired,
            ErrorCode::GeoRestricted,
            ErrorCode::BackendProtocol,
        ] {
            assert!(!triggers_fallback(code), "{code:?} must not fall through");
        }
    }
}
