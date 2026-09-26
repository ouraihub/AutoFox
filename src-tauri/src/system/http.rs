//! Shared HTTP client configuration.

use std::sync::LazyLock;

/// Installs rustls' ring provider once for the process.
pub fn ensure_crypto_provider() {
    static PROVIDER: LazyLock<()> = LazyLock::new(|| {
        let _ = rustls::crypto::CryptoProvider::install_default(
            rustls::crypto::ring::default_provider(),
        );
    });
    LazyLock::force(&PROVIDER);
}
