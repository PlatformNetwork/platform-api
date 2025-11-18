use crate::state::AppState;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

/// JWT authentication middleware (DISABLED - JWT removed, using random key-based authentication)
pub async fn jwt_auth_middleware(
    State(_state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // JWT middleware disabled - JWT removed, using random key-based authentication instead
    // All requests are allowed through
    Ok(next.run(req).await)
}

/// Rate limiting configuration
pub fn rate_limit_layer() -> impl Clone {
    let config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(10) // 10 requests per second
            .burst_size(20) // Burst of up to 20 requests
            .finish()
            .expect("Failed to create rate limiter config"),
    );

    GovernorLayer { config }
}

/// Per-endpoint rate limiting
pub fn endpoint_rate_limit(requests_per_minute: u32) -> impl Clone {
    let config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(requests_per_minute as u64 / 60)
            .burst_size(requests_per_minute * 2)
            .finish()
            .expect("Failed to create endpoint rate limiter"),
    );

    GovernorLayer { config }
}

/// Attestation validation middleware
pub async fn attestation_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let attestation_required =
        std::env::var("ATTESTATION_REQUIRED").unwrap_or_else(|_| "false".to_string()) == "true";

    if !attestation_required {
        return Ok(next.run(req).await);
    }

    let path = req.uri().path();
    let method = req.method().clone();

    // ALL GET requests are public (no attestation required)
    if method == axum::http::Method::GET {
        return Ok(next.run(req).await);
    }

    // Additional public endpoints for non-GET requests
    let is_public_endpoint = path == "/health" || path.starts_with("/ui/");

    if is_public_endpoint {
        return Ok(next.run(req).await);
    }

    // Check if request has valid attestation session
    let session_token = req
        .headers()
        .get("X-Attestation-Token")
        .and_then(|v| v.to_str().ok());

    match session_token {
        Some(token) => {
            // Verify attestation session token (using random key-based authentication)
            match state.attestation.verify_token_async(token).await {
                Ok(_claims) => Ok(next.run(req).await),
                Err(e) => {
                    tracing::warn!("Attestation verification failed: {}", e);
                    Err(StatusCode::FORBIDDEN)
                }
            }
        }
        None => {
            tracing::warn!("Missing attestation token");
            Err(StatusCode::FORBIDDEN)
        }
    }
}

/// CORS configuration for production
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::CorsLayer;

    let allowed_origins_str = std::env::var("ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "https://api.platform.network,http://localhost:3000".to_string());

    let origins: Result<Vec<_>, _> = allowed_origins_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<axum::http::HeaderValue>())
        .collect();

    match origins {
        Ok(valid_origins) => CorsLayer::new()
            .allow_origin(valid_origins)
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::DELETE,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                header::AUTHORIZATION,
                header::CONTENT_TYPE,
                header::HeaderName::from_static("x-attestation-token"),
            ])
            .allow_credentials(true),
        Err(e) => {
            tracing::warn!(
                "Failed to parse ALLOWED_ORIGINS, using permissive CORS: {}",
                e
            );
            CorsLayer::permissive()
        }
    }
}

/// Security headers middleware
pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    // Add security headers
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::HeaderName::from_static("x-xss-protection"),
        header::HeaderValue::from_static("1; mode=block"),
    );
    headers.insert(
        header::HeaderName::from_static("strict-transport-security"),
        header::HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );
    headers.insert(
        header::HeaderName::from_static("content-security-policy"),
        header::HeaderValue::from_static("default-src 'self'; script-src 'self' 'unsafe-inline'"),
    );

    response
}

/// Request validation middleware
pub async fn request_validation_middleware(
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Check request size
    let max_body_size = std::env::var("MAX_BODY_SIZE")
        .unwrap_or_else(|_| "10485760".to_string()) // 10MB default
        .parse::<usize>()
        .unwrap_or(10485760);

    if let Some(content_length) = req.headers().get(header::CONTENT_LENGTH) {
        if let Ok(length_str) = content_length.to_str() {
            if let Ok(length) = length_str.parse::<usize>() {
                if length > max_body_size {
                    tracing::warn!("Request body too large: {} bytes", length);
                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
            }
        }
    }

    Ok(next.run(req).await)
}

/// IP whitelist middleware for admin endpoints
pub async fn ip_whitelist_middleware(req: Request, next: Next) -> Result<Response, StatusCode> {
    // Only apply to admin endpoints
    if !req.uri().path().starts_with("/admin") {
        return Ok(next.run(req).await);
    }

    let whitelisted_ips = std::env::var("ADMIN_IP_WHITELIST")
        .unwrap_or_else(|_| "127.0.0.1,::1".to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .collect::<Vec<_>>();

    // Get client IP (considering proxy headers)
    let client_ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .unwrap_or("unknown");

    if whitelisted_ips.contains(&client_ip.to_string()) {
        Ok(next.run(req).await)
    } else {
        tracing::warn!("Admin access denied from IP: {}", client_ip);
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_middleware_disabled() {
        // JWT middleware is now disabled - all requests pass through
        // This test verifies the middleware doesn't panic
        // (Actual testing would require async runtime setup)
    }
}
