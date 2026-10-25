//! Extractor backend abstraction and the fallback boundary from
//! `docs/03-后端契约.md` §1 and §5.
//!
//! The download engine never learns which backend produced a `MediaManifest`.
//! That is what lets a native Rust YouTube extractor land beside the bundled
//! yt-dlp process without touching the download path.
//!
//! [`Extractor`] is the seam. [`probe_with_fallback`] walks candidates in
//! priority order and is the only place that decides whether a failure ends
//! the attempt or moves to the next backend.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use url::Url;

use super::MediaManifest;
use crate::ipc::{ErrorCode, IpcError};

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

/// Id of the native Rust extractor, which no build ships yet.
pub const NATIVE: BackendId = BackendId("native");

/// Future returned by [`Extractor::probe`].
///
/// Hand-boxed rather than `async fn` in the trait: AutoFox selects backends at
/// runtime from settings, so the trait has to stay `dyn`-compatible, and
/// `async fn` in a trait is not. Boxing once per probe is irrelevant next to
/// spawning a process.
pub type ProbeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<MediaManifest, IpcError>> + Send + 'a>>;

/// One interchangeable source of normalized media metadata.
///
/// Implementors own their own configuration. `probe` therefore takes only the
/// URL, which is what keeps [`probe_with_fallback`] free of any knowledge about
/// executables, paths, or settings.
pub trait Extractor: Send + Sync {
    /// Stable identity, used for logging and for reporting `producedBy`.
    fn id(&self) -> BackendId;

    /// Extracts a normalized manifest for `url`.
    fn probe<'a>(&'a self, url: &'a Url) -> ProbeFuture<'a>;
}

/// Whether a backend error may be retried on the next candidate.
///
/// §5 of the contract: only `unsupported` and `extractionFailed` fall through.
/// Falling through on `network` or `timedOut` would serialize one timeout per
/// backend on an offline machine and report "everything failed" instead of the
/// real cause.
#[must_use]
pub fn triggers_fallback(code: ErrorCode) -> bool {
    matches!(code, ErrorCode::Unsupported | ErrorCode::ExtractionFailed)
}

/// Probes `url` against each candidate until one succeeds.
///
/// Returns the **first** error when every candidate falls through, not the
/// last: the first backend is the most specific one for the URL, so its
/// `unsupported` detail is the diagnosis worth showing. A non-fallthrough
/// error stops the walk immediately.
///
/// An empty candidate list is a caller bug rather than a user-visible state,
/// but it still returns `NoBackendAvailable` instead of panicking: the policy
/// that produced the list comes from persisted settings.
pub async fn probe_with_fallback(
    candidates: &[&dyn Extractor],
    url: &Url,
) -> Result<MediaManifest, IpcError> {
    let mut first_error: Option<IpcError> = None;

    for candidate in candidates {
        crate::logs::debug(
            super::TARGET,
            format!("candidate backend {}", candidate.id()),
        );
        match candidate.probe(url).await {
            Ok(manifest) => return Ok(manifest),
            Err(error) => {
                if !triggers_fallback(error.code) {
                    return Err(error);
                }
                crate::logs::warn(
                    super::TARGET,
                    format!("backend {} fell through: {}", candidate.id(), error.detail),
                );
                first_error = first_error.or(Some(error));
            }
        }
    }

    Err(first_error.unwrap_or_else(|| IpcError {
        code: ErrorCode::NoBackendAvailable,
        message_key: "errors.noBackendAvailable",
        field: None,
        retryable: false,
        detail: "no extractor backend is configured".to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::{
        BackendId, ErrorCode, Extractor, IpcError, MediaManifest, ProbeFuture, probe_with_fallback,
        triggers_fallback,
    };
    use url::Url;

    /// Records whether it ran, so a test can prove a later backend was never
    /// consulted rather than merely that the final error looks right.
    ///
    /// `AtomicBool` rather than `RefCell`: `Extractor` requires `Sync`, and a
    /// test double that needs `unsafe impl Sync` to satisfy the trait is a test
    /// double that proves the trait is awkward for real implementors too.
    struct StubBackend {
        id: BackendId,
        outcome: Option<ErrorCode>,
        probed: AtomicBool,
    }

    impl StubBackend {
        fn failing(id: &'static str, code: ErrorCode) -> Self {
            Self {
                id: BackendId(id),
                outcome: Some(code),
                probed: AtomicBool::new(false),
            }
        }

        fn succeeding(id: &'static str) -> Self {
            Self {
                id: BackendId(id),
                outcome: None,
                probed: AtomicBool::new(false),
            }
        }

        fn was_probed(&self) -> bool {
            self.probed.load(Ordering::Relaxed)
        }
    }

    impl Extractor for StubBackend {
        fn id(&self) -> BackendId {
            self.id
        }

        fn probe<'a>(&'a self, _url: &'a Url) -> ProbeFuture<'a> {
            Box::pin(async move {
                self.probed.store(true, Ordering::Relaxed);
                match self.outcome {
                    None => Ok(MediaManifest {
                        source_id: "stub".to_owned(),
                        title: "stub".to_owned(),
                        duration_secs: None,
                        formats: Vec::new(),
                        download_options: Vec::new(),
                        entries: Vec::new(),
                        produced_by: self.id.0.to_owned(),
                        backend_version: "test".to_owned(),
                    }),
                    Some(code) => Err(IpcError {
                        code,
                        message_key: "errors.stub",
                        field: None,
                        retryable: false,
                        detail: format!("{} failed", self.id),
                    }),
                }
            })
        }
    }

    fn url() -> Url {
        Url::parse("https://example.com/watch?v=x").expect("valid fixture URL")
    }

    #[tokio::test]
    async fn unsupported_falls_through_to_the_next_backend() {
        let first = StubBackend::failing("a", ErrorCode::Unsupported);
        let second = StubBackend::succeeding("b");
        let candidates: [&dyn Extractor; 2] = [&first, &second];

        let manifest = probe_with_fallback(&candidates, &url())
            .await
            .expect("second backend should answer");

        assert_eq!(manifest.produced_by, "b");
        assert!(second.was_probed(), "fallback must reach the next backend");
    }

    /// The boundary the contract calls the easiest thing to get wrong: an
    /// offline machine must fail once with `network`, not once per backend.
    #[tokio::test]
    async fn network_failure_stops_the_walk_immediately() {
        let first = StubBackend::failing("a", ErrorCode::Network);
        let second = StubBackend::succeeding("b");
        let candidates: [&dyn Extractor; 2] = [&first, &second];

        let error = probe_with_fallback(&candidates, &url())
            .await
            .expect_err("network must not fall through");

        assert_eq!(error.code, ErrorCode::Network);
        assert!(
            !second.was_probed(),
            "a network failure must not serialize a second attempt"
        );
    }

    /// The first candidate is the most specific one for the URL, so its
    /// diagnosis is the one worth surfacing when everything falls through.
    #[tokio::test]
    async fn reports_the_first_diagnosis_when_all_candidates_fall_through() {
        let first = StubBackend::failing("a", ErrorCode::Unsupported);
        let second = StubBackend::failing("b", ErrorCode::ExtractionFailed);
        let candidates: [&dyn Extractor; 2] = [&first, &second];

        let error = probe_with_fallback(&candidates, &url())
            .await
            .expect_err("both candidates fail");

        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(error.detail, "a failed");
        assert!(second.was_probed(), "every candidate must be tried");
    }

    #[tokio::test]
    async fn empty_candidate_list_reports_no_backend_available() {
        let error = probe_with_fallback(&[], &url())
            .await
            .expect_err("no candidates means no manifest");
        assert_eq!(error.code, ErrorCode::NoBackendAvailable);
    }

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
