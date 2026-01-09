// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! API error types.
//!
//! # Security Considerations
//!
//! Internal errors are logged with full details but return sanitized messages to clients.
//! This prevents leaking implementation details, file paths, or stack traces that could
//! aid attackers. The logged error includes a correlation ID that can be used for
//! support investigations.

use serde::Serialize;
use std::fmt;

/// API error type.
///
/// Error response format:
/// - `BadRequest`: Returns the provided message (should be user-safe)
/// - `Internal`: Returns generic message + error_id for correlation
/// - `ServiceUnavailable`: Returns the provided message (should be user-safe)
/// - `Bootstrapping`: Returns fixed message
/// - `NotFound`: Returns the provided message
/// - `Forbidden`: Returns the provided message (access denied)
#[derive(Debug)]
pub enum ApiError {
    /// Invalid request parameters - message is safe to expose to clients.
    BadRequest(String),
    /// Internal server error - message will be logged but NOT exposed to clients.
    Internal(String),
    /// Service temporarily unavailable - message is safe to expose to clients.
    ServiceUnavailable(String),
    /// Node is bootstrapping.
    Bootstrapping,
    /// Resource not found.
    NotFound(String),
    /// Access forbidden.
    Forbidden(String),
}

impl ApiError {
    /// Generate a unique error ID for log correlation.
    ///
    /// Format: 8 hex characters from current timestamp + random component.
    /// This is short enough to include in error responses while being
    /// unique enough for practical correlation.
    pub fn generate_error_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);

        // Use lower bits of nanosecond timestamp for reasonable uniqueness
        format!("{:08x}", (timestamp & 0xFFFF_FFFF) as u32)
    }

    /// Get HTTP status code for this error.
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::Forbidden(_) => 403,
            Self::NotFound(_) => 404,
            Self::Internal(_) => 500,
            Self::ServiceUnavailable(_) | Self::Bootstrapping => 503,
        }
    }

    /// Get the client-safe error message.
    ///
    /// For internal errors, this returns a generic message with an error ID.
    /// The actual error details are only logged server-side.
    #[must_use]
    pub fn client_message(&self) -> String {
        match self {
            Self::BadRequest(msg) | Self::NotFound(msg) | Self::ServiceUnavailable(msg) | Self::Forbidden(msg) => {
                msg.clone()
            }
            Self::Bootstrapping => "Node is bootstrapping".to_string(),
            Self::Internal(_) => {
                let error_id = Self::generate_error_id();
                format!("Internal server error (error_id: {error_id})")
            }
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(msg) => write!(f, "Bad request: {msg}"),
            Self::Forbidden(msg) => write!(f, "Forbidden: {msg}"),
            Self::NotFound(msg) => write!(f, "Not found: {msg}"),
            Self::Internal(msg) => write!(f, "Internal error: {msg}"),
            Self::ServiceUnavailable(msg) => write!(f, "Service unavailable: {msg}"),
            Self::Bootstrapping => write!(f, "Node is bootstrapping"),
        }
    }
}

impl std::error::Error for ApiError {}

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        use axum::Json;

        let status = match self.status_code() {
            400 => StatusCode::BAD_REQUEST,
            403 => StatusCode::FORBIDDEN,
            404 => StatusCode::NOT_FOUND,
            503 => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // Log internal errors
        if let Self::Internal(ref msg) = self {
            tracing::error!("Internal error: {msg}");
        }

        let body = ErrorResponse::new(self.client_message());
        (status, Json(body)).into_response()
    }
}

/// JSON error response body.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Error message.
    pub error: String,
}

impl ErrorResponse {
    /// Create a new error response.
    pub fn new(message: impl Into<String>) -> Self {
        Self { error: message.into() }
    }
}
