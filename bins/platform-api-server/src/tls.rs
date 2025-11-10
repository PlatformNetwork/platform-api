use anyhow::{Context, Result};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use std::net::SocketAddr;

/// Serve HTTPS with TLS
pub async fn serve_https(
    router: Router,
    addr: SocketAddr,
    cert_path: &str,
    key_path: &str,
) -> Result<()> {
    // Set up rustls crypto provider
    // Note: install_default() may panic if called multiple times or if provider is already set
    // In rustls 0.23, the default provider is usually set automatically, so we skip explicit installation
    // If needed, the provider can be installed once at application startup instead of here
    // For now, we rely on rustls using the default provider configured via Cargo features (aws_lc_rs)

    tracing::info!(
        "🔒 Loading TLS certificates from {} and {}",
        cert_path,
        key_path
    );

    // Load TLS configuration
    let config = RustlsConfig::from_pem_file(cert_path, key_path)
        .await
        .context("Failed to load TLS configuration")?;

    tracing::info!("🔒 HTTPS server listening on {}", addr);

    // Serve with axum-server
    axum_server::bind_rustls(addr, config)
        .serve(router.into_make_service())
        .await
        .context("Failed to serve HTTPS")?;

    Ok(())
}
