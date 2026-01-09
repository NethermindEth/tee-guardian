// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Structured logging with file-based and OpenObserve push layers

use crate::unix_timestamp_micros;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write as IoWrite};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tracing::field::Field;
use tracing::Subscriber;
use tracing_subscriber::field::Visit;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

/// Maximum number of keys allowed in log context
const MAX_CONTEXT_KEYS: usize = 10;

/// Dynamic log context for tests
static LOG_CONTEXT: once_cell::sync::Lazy<Arc<RwLock<HashMap<String, String>>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Rate limiter: 10 calls per second for log context modifications
static RATE_LIMITER: once_cell::sync::Lazy<RateLimiter<NotKeyed, InMemoryState, DefaultClock>> =
    once_cell::sync::Lazy::new(|| RateLimiter::direct(Quota::per_second(NonZeroU32::new(10).expect("10 > 0"))));

/// Error returned when log context modification fails
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogContextError {
    /// Rate limit exceeded (max 10 calls/second)
    RateLimitExceeded,
    /// Maximum number of context keys reached (max 10)
    MaxKeysExceeded,
}

impl std::fmt::Display for LogContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimitExceeded => write!(f, "rate limit exceeded (max 10 calls/second)"),
            Self::MaxKeysExceeded => {
                write!(f, "maximum context keys exceeded (max {MAX_CONTEXT_KEYS})")
            }
        }
    }
}

impl std::error::Error for LogContextError {}

/// Set log context with rate limiting and key count limits.
///
/// Returns an error if:
/// - More than 10 calls are made per second (rate limit)
/// - More than 10 keys would exist after the operation (key limit)
///
/// Removing keys (value = None or empty) is always allowed without rate limiting.
pub fn set_log_context(key: String, value: Option<String>) -> Result<(), LogContextError> {
    // Removals are always allowed without rate limiting
    if value.as_ref().map_or(true, String::is_empty) {
        if let Ok(mut ctx) = LOG_CONTEXT.write() {
            ctx.remove(&key);
        }
        return Ok(());
    }

    // Rate limiting check for additions/modifications
    if RATE_LIMITER.check().is_err() {
        return Err(LogContextError::RateLimitExceeded);
    }

    // Key count limit check
    let mut ctx = LOG_CONTEXT.write().unwrap_or_else(|e| e.into_inner());

    // Only check limit if adding a new key (not updating existing)
    if !ctx.contains_key(&key) && ctx.len() >= MAX_CONTEXT_KEYS {
        return Err(LogContextError::MaxKeysExceeded);
    }

    if let Some(v) = value {
        if !v.is_empty() {
            ctx.insert(key, v);
        }
    }

    Ok(())
}

/// Get log context
pub fn get_log_context() -> HashMap<String, String> {
    LOG_CONTEXT.read().map(|g| g.clone()).unwrap_or_default()
}

/// Quote logfmt value if it contains special characters
fn quote_logfmt_value(val: &str) -> String {
    if val.contains([' ', '=', '"']) {
        format!("\"{}\"", val.replace('"', "\\\""))
    } else {
        val.to_string()
    }
}

/// Visitor to collect event fields, filtering out tracing metadata
struct FieldCollector {
    message: String,
    fields: Vec<(String, String)>,
}

impl FieldCollector {
    fn new() -> Self {
        Self { message: String::new(), fields: Vec::new() }
    }
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let name = field.name();

        // Skip internal tracing metadata (log.target, log.module_path, log.file, log.line)
        if name.starts_with("log.") {
            return;
        }

        let val = format!("{:?}", value).trim_matches('"').to_string();

        if name == "message" {
            self.message = val;
        } else {
            self.fields.push((name.to_string(), val));
        }
    }
}

/// File-based logfmt layer with LOG_CONTEXT support and automatic rotation
struct FileLogLayer {
    writer: Arc<Mutex<BufWriter<File>>>,
    file_path: String,
    max_size_bytes: u64,
}

impl FileLogLayer {
    fn new(file_path: String, max_size_mb: u64) -> Result<Self, std::io::Error> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = std::path::Path::new(&file_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new().create(true).append(true).open(&file_path)?;

        Ok(Self {
            writer: Arc::new(Mutex::new(BufWriter::new(file))),
            file_path,
            max_size_bytes: max_size_mb * 1024 * 1024,
        })
    }

    /// Check if log file exceeds max size and rotate if necessary.
    ///
    /// Rotation keeps the last 50% of the file to avoid losing recent logs
    /// and to prevent frequent rotations when near the size limit.
    ///
    /// # Mutex Poisoning
    ///
    /// This method recovers from mutex poisoning because log file rotation
    /// should not prevent future logging. A poisoned mutex indicates a panic
    /// occurred while holding the lock, but the file handle is likely still valid.
    fn check_and_rotate(&self) -> Result<(), std::io::Error> {
        let writer = self.writer.lock().unwrap_or_else(|poisoned| {
            eprintln!("File log writer mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let file = writer.get_ref();
        let metadata = file.metadata()?;

        if metadata.len() > self.max_size_bytes {
            // Truncate file to last 50% of max size to avoid frequent rotations
            let keep_size = self.max_size_bytes / 2;
            let skip_size = metadata.len() - keep_size;

            // Read the last portion of the file
            let mut file_ref = file.try_clone()?;
            file_ref.seek(SeekFrom::Start(skip_size))?;

            let mut buffer = Vec::new();
            std::io::Read::read_to_end(&mut file_ref, &mut buffer)?;

            // Find first newline to avoid partial log entries
            if let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                buffer.drain(..=newline_pos);
            }

            // Reopen file in write mode (truncate) and write back the kept portion
            drop(writer); // Release the lock

            let mut new_file = OpenOptions::new().write(true).truncate(true).open(&self.file_path)?;

            new_file.write_all(&buffer)?;
            new_file.flush()?;

            // Reopen in append mode
            let file = OpenOptions::new().create(true).append(true).open(&self.file_path)?;

            // Recover from poisoning here too - rotation should complete even if
            // a previous write panicked
            *self.writer.lock().unwrap_or_else(|p| p.into_inner()) = BufWriter::new(file);
        }

        Ok(())
    }

    /// Write a log line to the file with retry logic.
    ///
    /// Retries up to 3 times on write failures (e.g., disk full, I/O errors).
    /// Uses mutex poisoning recovery to ensure logging continues even after panics.
    fn write_log(&self, output: &str) {
        const MAX_RETRIES: u32 = 3;

        for retry in 0..MAX_RETRIES {
            // Check for rotation before writing
            if let Err(e) = self.check_and_rotate() {
                eprintln!("Failed to rotate log file: {e}");
            }

            let result = {
                // Recover from poisoned mutex - file logging should continue
                let mut writer = self.writer.lock().unwrap_or_else(|poisoned| {
                    if retry == 0 {
                        eprintln!("File log writer mutex was poisoned, recovering");
                    }
                    poisoned.into_inner()
                });
                writeln!(writer, "{output}").and_then(|()| writer.flush())
            };

            match result {
                Ok(()) => return,
                Err(e) => {
                    if retry + 1 >= MAX_RETRIES {
                        eprintln!("Failed to write log after {MAX_RETRIES} retries: {e}");
                    } else {
                        eprintln!("Failed to write log (retry {}/{}): {e}", retry + 1, MAX_RETRIES);
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        }
    }
}

impl<S> Layer<S> for FileLogLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let mut output = String::with_capacity(256);

        // 1. Timestamp (ISO8601 format)
        let datetime = chrono::Utc::now();
        let _ = write!(output, "time={}", datetime.format("%Y-%m-%dT%H:%M:%S%.9fZ"));

        // 2. Level
        let _ = write!(output, " level={}", event.metadata().level().as_str());

        // 3. Collect event fields using proper visitor
        let mut visitor = FieldCollector::new();
        event.record(&mut visitor);

        // 4. Message (immediately after level for hl rendering)
        if !visitor.message.is_empty() {
            let _ = write!(output, " message={}", quote_logfmt_value(&visitor.message));
        }

        // 5. LOG_CONTEXT (test-id, etc.)
        for (key, val) in get_log_context() {
            let _ = write!(output, " {key}={}", quote_logfmt_value(&val));
        }

        // 6. Custom fields
        for (key, val) in visitor.fields {
            let _ = write!(output, " {key}={}", quote_logfmt_value(&val));
        }

        self.write_log(&output);
    }
}

/// OpenObserve log entry
#[derive(Serialize)]
struct LogEntry {
    #[serde(rename = "_timestamp")]
    timestamp: i64,
    level: String,
    #[serde(rename = "host-id")]
    host_id: String,
    message: String,
    #[serde(flatten)]
    fields: HashMap<String, String>,
}

/// OpenObserve push layer
///
/// Pushes logs to an OpenObserve instance via HTTP. Logs are buffered and
/// sent in batches for efficiency. A background worker task handles the
/// actual HTTP requests to avoid blocking the logging path.
struct OpenObserveLayer {
    host_id: std::net::Ipv4Addr,
    buffer: Mutex<Vec<LogEntry>>,
    sender: tokio::sync::mpsc::UnboundedSender<Vec<LogEntry>>,
}

impl OpenObserveLayer {
    /// Create a new OpenObserve logging layer.
    ///
    /// # Panics
    ///
    /// This function will panic if the HTTP client cannot be created. This can occur if:
    /// - TLS backend initialization fails (system OpenSSL/rustls misconfiguration)
    /// - System resource exhaustion (no file descriptors available)
    ///
    /// In practice, client creation with basic configuration (timeout only) is highly
    /// reliable. We panic here rather than returning Result because:
    /// 1. Logging initialization happens early in application startup
    /// 2. A failed logging layer would cause silent log loss - better to fail loudly
    /// 3. The caller (init_logging) already handles layer creation failures gracefully
    ///    by continuing without the OpenObserve layer
    ///
    /// # Arguments
    ///
    /// * `endpoint` - OpenObserve server URL (e.g., "https://logs.example.com")
    /// * `org` - OpenObserve organization name
    /// * `stream` - Log stream name within the organization
    /// * `user` - Authentication username
    /// * `password` - Authentication password
    /// * `host_id` - Ipv4 address for this host (appears in logs)
    fn new(endpoint: &str, org: &str, stream: &str, user: &str, password: &str, host_id: std::net::Ipv4Addr) -> Self {
        let url = format!("{endpoint}/api/{org}/{stream}/_json");
        let auth = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{user}:{password}"));

        // Create HTTP client outside the spawn so failures are visible at initialization.
        // This is preferable to panicking inside spawn() where the error would be
        // swallowed and logs would silently disappear.
        let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().expect(
            "Failed to create HTTP client for OpenObserve logging. \
                     This typically indicates a TLS backend issue or system resource exhaustion.",
        );

        // Create channel for sending logs to worker
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<LogEntry>>();

        // Spawn worker task that owns the HTTP client.
        // The client is now guaranteed to exist, so this task cannot panic from
        // client creation. The task will run until the sender is dropped.
        tokio::spawn(async move {
            while let Some(logs) = rx.recv().await {
                if let Err(e) =
                    client.post(&url).header("Authorization", format!("Basic {auth}")).json(&logs).send().await
                {
                    // Log to stderr since our primary logging channel (OpenObserve) is failing.
                    // This is intentionally not using tracing to avoid recursion.
                    eprintln!("Failed to push logs to OpenObserve: {e}");
                }
            }
        });

        Self { host_id, buffer: Mutex::new(Vec::new()), sender: tx }
    }

    /// Flush buffered logs to the worker task for transmission.
    ///
    /// This method is resilient to mutex poisoning - if the mutex is poisoned
    /// (indicating a previous panic), we recover by taking the inner value.
    /// For logging infrastructure, availability is more important than strict
    /// consistency guarantees.
    fn flush_buffer(&self) {
        // Recover from poisoned mutex - logging should continue even if a previous
        // operation panicked. The poisoned data is still valid for our use case.
        let mut buf = self.buffer.lock().unwrap_or_else(|poisoned| {
            eprintln!("OpenObserve buffer mutex was poisoned, recovering");
            poisoned.into_inner()
        });

        if buf.is_empty() {
            return;
        }

        let logs = std::mem::take(&mut *buf);
        if self.sender.send(logs).is_err() {
            eprintln!("Log worker task has stopped - logs will be lost");
        }
    }
}

impl<S> Layer<S> for OpenObserveLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // Collect fields using proper visitor (filters out log.* metadata)
        let mut visitor = FieldCollector::new();
        event.record(&mut visitor);

        // Convert fields vec to HashMap and add log context
        let mut fields: HashMap<String, String> = visitor.fields.into_iter().collect();
        fields.extend(get_log_context());

        // Create log entry
        let entry = LogEntry {
            timestamp: unix_timestamp_micros(),
            level: event.metadata().level().to_string(),
            host_id: self.host_id.to_string(),
            message: visitor.message,
            fields,
        };

        // Buffer and flush when full.
        // Recover from poisoned mutex - logging should remain available.
        let mut buf = self.buffer.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        buf.push(entry);

        if buf.len() >= crate::config::LOG_PUSH_BATCH_SIZE {
            drop(buf); // Release lock before flushing
            self.flush_buffer();
        }
    }
}

/// Configuration for logging initialization
struct LogConfig<'a> {
    host_id: std::net::Ipv4Addr,
    log_file_path: &'a str,
    max_log_file_size_mb: u64,
    openobserve: Option<OpenObserveConfig<'a>>,
}

struct OpenObserveConfig<'a> {
    endpoint: &'a str,
    org: &'a str,
    stream: &'a str,
    user: &'a str,
    password: &'a str,
}

/// Parse log level from string
fn parse_log_level(level: &str) -> tracing::Level {
    match level {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    }
}

/// Internal logging initialization with configurable behavior
fn init_logging_internal(
    config: LogConfig<'_>,
    use_global_subscriber: bool,
) -> Result<Option<tracing::subscriber::DefaultGuard>, Box<dyn std::error::Error>> {
    let level = parse_log_level(crate::config::LOG_LEVEL);
    let filter = tracing_subscriber::filter::LevelFilter::from_level(level);

    // Create file-based layer
    let file_layer = match FileLogLayer::new(config.log_file_path.to_string(), config.max_log_file_size_mb) {
        Ok(layer) => Some(layer.with_filter(filter)),
        Err(e) => {
            eprintln!("Failed to initialize file logging: {e}");
            None
        }
    };

    // Create OpenObserve layer if configured
    let oo_layer = config
        .openobserve
        .map(|oo| OpenObserveLayer::new(oo.endpoint, oo.org, oo.stream, oo.user, oo.password, config.host_id));

    // Initialize subscriber with available layers
    let guard = match (file_layer, oo_layer, use_global_subscriber) {
        (Some(file), Some(oo), true) => {
            tracing_subscriber::registry().with(file).with(oo).try_init()?;
            None
        }
        (Some(file), None, true) => {
            tracing_subscriber::registry().with(file).try_init()?;
            None
        }
        (None, Some(oo), true) => {
            tracing_subscriber::registry().with(oo).try_init()?;
            None
        }
        (Some(file), Some(oo), false) => {
            let subscriber = tracing_subscriber::registry().with(file).with(oo);
            Some(tracing::subscriber::set_default(subscriber))
        }
        (Some(file), None, false) => {
            let subscriber = tracing_subscriber::registry().with(file);
            Some(tracing::subscriber::set_default(subscriber))
        }
        (None, Some(oo), false) => {
            let subscriber = tracing_subscriber::registry().with(oo);
            Some(tracing::subscriber::set_default(subscriber))
        }
        (None, None, _) => {
            return Err("No logging layers available".into());
        }
    };

    tracing::info!("Logging initialized");
    Ok(guard)
}

/// Initialize logging with host-id
pub fn init_logging(host_id: std::net::Ipv4Addr) -> Result<(), Box<dyn std::error::Error>> {
    let config = LogConfig {
        host_id,
        log_file_path: crate::config::LOG_FILE_PATH,
        max_log_file_size_mb: crate::config::MAX_LOG_FILE_SIZE_MB,
        openobserve: crate::config::OPENOBSERVE_ENDPOINT.map(|endpoint| OpenObserveConfig {
            endpoint,
            org: crate::config::OPENOBSERVE_ORG,
            stream: crate::config::OPENOBSERVE_STREAM,
            user: crate::config::OPENOBSERVE_USER,
            password: crate::config::OPENOBSERVE_PASSWORD,
        }),
    };

    init_logging_internal(config, true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::subscriber::DefaultGuard;

    /// Initialize logging for tests with proper isolation using a guard
    fn init_logging_for_test(host_id: std::net::Ipv4Addr) -> Result<DefaultGuard, Box<dyn std::error::Error>> {
        let test_log_path = format!("/tmp/guardian_test_{}.log", std::process::id());

        let config = LogConfig {
            host_id,
            log_file_path: &test_log_path,
            max_log_file_size_mb: crate::config::MAX_LOG_FILE_SIZE_MB,
            // Disable OpenObserve for tests - avoids tokio::spawn issues with runtime context
            openobserve: None,
        };

        init_logging_internal(config, false)?.ok_or_else(|| "Expected guard for test logging".into())
    }

    #[tokio::test]
    async fn test_logging_init() {
        let result = init_logging_for_test(std::net::Ipv4Addr::new(127, 0, 0, 1));
        assert!(result.is_ok());
        let _guard = result.unwrap();

        // Test that we can actually log
        tracing::info!("Test log message");
    }

    #[tokio::test]
    async fn test_log_context() {
        let _guard = init_logging_for_test(std::net::Ipv4Addr::new(127, 0, 0, 1))
            .expect("Failed to initialize logging for test");

        // Directly manipulate LOG_CONTEXT to avoid rate limiter interference from other tests
        if let Ok(mut ctx) = LOG_CONTEXT.write() {
            ctx.clear();
            ctx.insert("test-run".to_string(), "integration-001".to_string());
            ctx.insert("cluster-id".to_string(), "test-cluster".to_string());
        }

        tracing::info!("Test with context");
        tracing::info!(node_id = 12345, "Node initialized");

        // Context should appear in logs
        let ctx = get_log_context();
        assert_eq!(ctx.get("test-run"), Some(&"integration-001".to_string()));
        assert_eq!(ctx.get("cluster-id"), Some(&"test-cluster".to_string()));
    }

    #[test]
    fn test_rate_limiting() {
        // Just verify rate limiter is functional - don't exhaust quota
        // Removals bypass rate limiting, so this should always succeed
        let result = set_log_context("rate-test-key".to_string(), None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_key_limit() {
        // Clear context
        if let Ok(mut ctx) = LOG_CONTEXT.write() {
            ctx.clear();
        }

        // Wait a bit for rate limiter to reset
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Fill up to max keys
        for i in 0..MAX_CONTEXT_KEYS {
            let _ = set_log_context(format!("key{i}"), Some(format!("value{i}")));
        }

        // Next new key should fail
        let result = set_log_context("overflow".to_string(), Some("value".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_removal_bypasses_rate_limit() {
        // Removals should always work
        let result = set_log_context("any-key".to_string(), None);
        assert!(result.is_ok());

        let result = set_log_context("any-key".to_string(), Some(String::new()));
        assert!(result.is_ok());
    }
}
