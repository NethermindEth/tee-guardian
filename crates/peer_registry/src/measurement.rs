// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Measurement registry client for querying trusted RTMR measurements.
//!
//! The measurement registry is an external service that maintains a whitelist
//! of trusted RTMR measurement hashes. Each namespace (e.g., "guardian", "k8s")
//! has its own set of allowed measurements.
//!
//! # API Endpoints
//!
//! ```text
//! GET /measurements/{namespace}/{hash} -> 200 OK (trusted) or 404 (not trusted)
//! GET /measurements/{namespace}        -> List all trusted hashes
//! GET /nodes?namespace={namespace}     -> Bootstrap node discovery
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use peer_registry::MeasurementRegistryClient;
//!
//! let client = MeasurementRegistryClient::new("http://registry:9000".to_string());
//!
//! // Check if a measurement is trusted
//! let is_trusted = client.verify_measurement("guardian", "abc123...").await?;
//!
//! // Discover bootstrap nodes
//! let nodes = client.get_nodes("guardian").await?;
//! ```

use crate::types::{BootstrapNode, MeasurementRegistryError, NodeDiscoveryResponse};
use std::time::Duration;
use tracing::debug;

/// Default timeout for registry requests.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default number of retries for transient failures.
const DEFAULT_RETRIES: u32 = 3;

/// Base delay between retries (exponential backoff).
const RETRY_BASE_DELAY: Duration = Duration::from_millis(500);

/// HTTP client for the measurement registry service.
///
/// The registry maintains a whitelist of trusted RTMR measurement hashes.
/// Nodes must have measurements in the registry to join the cluster.
///
/// # Retry Behavior
///
/// Transient failures (connection errors, timeouts, 5xx errors) are
/// automatically retried with exponential backoff.
pub struct MeasurementRegistryClient {
    /// Registry base URL (e.g., "http://registry.example.com:9000")
    base_url: String,
    /// HTTP client with connection pooling
    client: reqwest::Client,
    /// Maximum number of retries for transient failures
    max_retries: u32,
}

impl MeasurementRegistryClient {
    /// Create a new registry client with default settings.
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL of the measurement registry (e.g., "http://registry:9000")
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be created (should never happen with valid settings).
    #[must_use]
    pub fn new(base_url: String) -> Self {
        Self::with_config(base_url, DEFAULT_TIMEOUT, DEFAULT_RETRIES)
    }

    /// Create a new registry client with custom configuration.
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL of the measurement registry
    /// * `timeout` - Request timeout duration
    /// * `max_retries` - Maximum number of retries for transient failures
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be created.
    #[must_use]
    pub fn with_config(base_url: String, timeout: Duration, max_retries: u32) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(5)
            .build()
            .expect("Failed to create HTTP client");

        Self { base_url, client, max_retries }
    }

    /// Query if a measurement hash is in the trusted whitelist.
    ///
    /// # Arguments
    ///
    /// * `namespace` - Namespace to check (e.g., "guardian", "k8s")
    /// * `measurement_hash` - SHA256 hash of RTMRs (hex-encoded)
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Measurement is trusted
    /// * `Ok(false)` - Measurement is not in the whitelist
    /// * `Err(_)` - Communication error with registry
    pub async fn verify_measurement(
        &self,
        namespace: &str,
        measurement_hash: &str,
    ) -> Result<bool, MeasurementRegistryError> {
        let url = format!("{}/measurements/{}/{}", self.base_url, namespace, measurement_hash);

        debug!(url = %url, "Querying measurement registry");

        let response = self.request_with_retry(&url).await?;

        if response.status().is_success() {
            Ok(true)
        } else if response.status() == reqwest::StatusCode::NOT_FOUND {
            Ok(false)
        } else {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            Err(MeasurementRegistryError::RegistryError { status, message: body })
        }
    }

    /// Get all measurements for a namespace.
    ///
    /// # Arguments
    ///
    /// * `namespace` - Namespace to query
    ///
    /// # Returns
    ///
    /// A vector of measurement entries (structure depends on registry implementation).
    pub async fn get_measurements(&self, namespace: &str) -> Result<Vec<serde_json::Value>, MeasurementRegistryError> {
        let url = format!("{}/measurements/{}", self.base_url, namespace);

        let response = self.request_with_retry(&url).await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(MeasurementRegistryError::RegistryError { status, message: body });
        }

        response.json().await.map_err(|e| MeasurementRegistryError::ParseError(e.to_string()))
    }

    /// Get bootstrap nodes registered for a namespace.
    ///
    /// # Arguments
    ///
    /// * `namespace` - Namespace to query (e.g., "guardian")
    ///
    /// # Returns
    ///
    /// A vector of bootstrap nodes. Returns empty vector if namespace has no nodes.
    pub async fn get_nodes(&self, namespace: &str) -> Result<Vec<BootstrapNode>, MeasurementRegistryError> {
        let url = format!("{}/nodes?namespace={}", self.base_url, namespace);

        debug!(url = %url, "Querying bootstrap nodes");

        let response = self.request_with_retry(&url).await?;

        if response.status().is_success() {
            let discovery_response: NodeDiscoveryResponse =
                response.json().await.map_err(|e| MeasurementRegistryError::ParseError(e.to_string()))?;

            Ok(discovery_response.nodes)
        } else if response.status() == reqwest::StatusCode::NOT_FOUND {
            // No nodes registered yet
            Ok(vec![])
        } else {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            Err(MeasurementRegistryError::RegistryError { status, message: body })
        }
    }

    /// Get the base URL of the registry.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Perform a GET request with automatic retry for transient failures.
    async fn request_with_retry(&self, url: &str) -> Result<reqwest::Response, MeasurementRegistryError> {
        let mut last_error = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                // Exponential backoff: 500ms, 1s, 2s, ...
                let delay = RETRY_BASE_DELAY * 2u32.saturating_pow(attempt - 1);
                tokio::time::sleep(delay).await;
                debug!(
                    attempt = attempt,
                    max_retries = self.max_retries,
                    delay_ms = delay.as_millis(),
                    "Retrying measurement registry request"
                );
            }

            match self.client.get(url).send().await {
                Ok(response) => {
                    // Retry on 5xx errors (server issues)
                    if response.status().is_server_error() && attempt < self.max_retries {
                        last_error = Some(MeasurementRegistryError::RegistryError {
                            status: response.status().as_u16(),
                            message: "Server error".to_string(),
                        });
                        continue;
                    }
                    return Ok(response);
                }
                Err(e) => {
                    last_error = Some(Self::classify_error(&e));
                    // Only retry transient errors
                    if !e.is_connect() && !e.is_timeout() {
                        break;
                    }
                }
            }
        }

        Err(last_error.expect("Should have at least one error after retry loop"))
    }

    /// Classify a reqwest error into our error type.
    fn classify_error(e: &reqwest::Error) -> MeasurementRegistryError {
        if e.is_connect() {
            MeasurementRegistryError::ConnectionFailed(e.to_string())
        } else if e.is_timeout() {
            MeasurementRegistryError::Timeout(e.to_string())
        } else {
            MeasurementRegistryError::ParseError(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_client() {
        let client = MeasurementRegistryClient::new("http://localhost:9000".to_string());
        assert_eq!(client.base_url(), "http://localhost:9000");
    }

    #[test]
    fn test_with_config() {
        let client =
            MeasurementRegistryClient::with_config("http://localhost:9000".to_string(), Duration::from_secs(10), 5);
        assert_eq!(client.max_retries, 5);
    }
}
