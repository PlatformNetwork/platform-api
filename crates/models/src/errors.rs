use thiserror::Error;

/// Platform API errors
#[derive(Error, Debug)]
pub enum PlatformError {
    #[error("Challenge not found: {id}")]
    ChallengeNotFound { id: String },

    #[error("Job not found: {id}")]
    JobNotFound { id: String },

    #[error("Invalid challenge configuration: {reason}")]
    InvalidChallengeConfig { reason: String },

    #[error("Invalid job configuration: {reason}")]
    InvalidJobConfig { reason: String },

    #[error("Attestation failed: {reason}")]
    AttestationFailed { reason: String },

    #[error("Key release failed: {reason}")]
    KeyReleaseFailed { reason: String },

    #[error("Storage error: {reason}")]
    StorageError { reason: String },

    #[error("Builder error: {reason}")]
    BuilderError { reason: String },

    #[error("Scheduler error: {reason}")]
    SchedulerError { reason: String },

    #[error("Configuration error: {reason}")]
    ConfigError { reason: String },

    #[error("Emission error: {reason}")]
    EmissionError { reason: String },

    #[error("Authentication failed: {reason}")]
    AuthenticationFailed { reason: String },

    #[error("Authorization failed: {reason}")]
    AuthorizationFailed { reason: String },

    #[error("Rate limit exceeded: {limit}")]
    RateLimitExceeded { limit: u32 },

    #[error("Validation error: {field} - {reason}")]
    ValidationError { field: String, reason: String },

    #[error("Serialization error: {reason}")]
    SerializationError { reason: String },

    #[error("Network error: {reason}")]
    NetworkError { reason: String },

    #[error("Database error: {reason}")]
    DatabaseError { reason: String },

    #[error("Internal server error: {reason}")]
    InternalError { reason: String },

    #[error("Service unavailable: {service}")]
    ServiceUnavailable { service: String },

    #[error("Timeout error: {operation}")]
    TimeoutError { operation: String },

    #[error("Resource not found: {resource}")]
    ResourceNotFound { resource: String },

    #[error("Resource already exists: {resource}")]
    ResourceAlreadyExists { resource: String },

    #[error("Invalid request: {reason}")]
    InvalidRequest { reason: String },

    #[error("Unsupported operation: {operation}")]
    UnsupportedOperation { operation: String },

    #[error("External service error: {service} - {reason}")]
    ExternalServiceError { service: String, reason: String },
}

impl PlatformError {
    /// Get HTTP status code for the error
    pub fn status_code(&self) -> u16 {
        match self {
            PlatformError::ChallengeNotFound { .. } => 404,
            PlatformError::JobNotFound { .. } => 404,
            PlatformError::ResourceNotFound { .. } => 404,
            PlatformError::InvalidChallengeConfig { .. } => 400,
            PlatformError::InvalidJobConfig { .. } => 400,
            PlatformError::ValidationError { .. } => 400,
            PlatformError::InvalidRequest { .. } => 400,
            PlatformError::AuthenticationFailed { .. } => 401,
            PlatformError::AuthorizationFailed { .. } => 403,
            PlatformError::ResourceAlreadyExists { .. } => 409,
            PlatformError::RateLimitExceeded { .. } => 429,
            PlatformError::TimeoutError { .. } => 408,
            PlatformError::ServiceUnavailable { .. } => 503,
            PlatformError::ExternalServiceError { .. } => 502,
            PlatformError::AttestationFailed { .. } => 422,
            PlatformError::KeyReleaseFailed { .. } => 422,
            PlatformError::StorageError { .. } => 500,
            PlatformError::BuilderError { .. } => 500,
            PlatformError::SchedulerError { .. } => 500,
            PlatformError::ConfigError { .. } => 500,
            PlatformError::EmissionError { .. } => 500,
            PlatformError::SerializationError { .. } => 500,
            PlatformError::NetworkError { .. } => 500,
            PlatformError::DatabaseError { .. } => 500,
            PlatformError::InternalError { .. } => 500,
            PlatformError::UnsupportedOperation { .. } => 501,
        }
    }

    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            PlatformError::NetworkError { .. }
                | PlatformError::DatabaseError { .. }
                | PlatformError::TimeoutError { .. }
                | PlatformError::ServiceUnavailable { .. }
                | PlatformError::ExternalServiceError { .. }
        )
    }

    /// Get error category
    pub fn category(&self) -> &'static str {
        match self {
            PlatformError::ChallengeNotFound { .. } => "challenge",
            PlatformError::JobNotFound { .. } => "job",
            PlatformError::InvalidChallengeConfig { .. } => "validation",
            PlatformError::InvalidJobConfig { .. } => "validation",
            PlatformError::AttestationFailed { .. } => "attestation",
            PlatformError::KeyReleaseFailed { .. } => "attestation",
            PlatformError::StorageError { .. } => "storage",
            PlatformError::BuilderError { .. } => "builder",
            PlatformError::SchedulerError { .. } => "scheduler",
            PlatformError::ConfigError { .. } => "config",
            PlatformError::EmissionError { .. } => "emission",
            PlatformError::AuthenticationFailed { .. } => "auth",
            PlatformError::AuthorizationFailed { .. } => "auth",
            PlatformError::RateLimitExceeded { .. } => "rate_limit",
            PlatformError::ValidationError { .. } => "validation",
            PlatformError::SerializationError { .. } => "serialization",
            PlatformError::NetworkError { .. } => "network",
            PlatformError::DatabaseError { .. } => "database",
            PlatformError::InternalError { .. } => "internal",
            PlatformError::ServiceUnavailable { .. } => "service",
            PlatformError::TimeoutError { .. } => "timeout",
            PlatformError::ResourceNotFound { .. } => "resource",
            PlatformError::ResourceAlreadyExists { .. } => "resource",
            PlatformError::InvalidRequest { .. } => "request",
            PlatformError::UnsupportedOperation { .. } => "operation",
            PlatformError::ExternalServiceError { .. } => "external",
        }
    }
}

/// Result type alias for Platform operations
pub type PlatformResult<T> = Result<T, PlatformError>;

/// Error response for API endpoints
#[derive(Debug, serde::Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub code: u16,
    pub category: String,
    pub retryable: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub request_id: Option<String>,
}

impl From<PlatformError> for ErrorResponse {
    fn from(err: PlatformError) -> Self {
        Self {
            error: err.category().to_string(),
            message: err.to_string(),
            code: err.status_code(),
            category: err.category().to_string(),
            retryable: err.is_retryable(),
            timestamp: chrono::Utc::now(),
            request_id: None,
        }
    }
}

impl From<anyhow::Error> for PlatformError {
    fn from(err: anyhow::Error) -> Self {
        PlatformError::InternalError {
            reason: err.to_string(),
        }
    }
}
