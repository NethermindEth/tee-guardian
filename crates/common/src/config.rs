// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Guardian Configuration - Compile-time constants

/// Measurement registry endpoint URL
pub const MEASUREMENT_REGISTRY_URL: &str = "http://measurement-registry:9000";
/// Measurement registry request timeout in seconds
pub const MEASUREMENT_REGISTRY_TIMEOUT_SECS: u64 = 30;
/// Interval for scanning measurement registry for revocations (seconds)
pub const MEASUREMENT_REVOCATION_SCAN_INTERVAL_SECS: u64 = 300; // 5 minutes

/// Guardian API bind address
pub const GUARDIAN_API_BIND_ADDR: &str = "0.0.0.0";
/// Guardian API port
pub const GUARDIAN_API_PORT: u16 = 8443;
/// Guardian API maximum request size in bytes
pub const GUARDIAN_API_MAX_REQUEST_SIZE: usize = 1024 * 1024; // 1MB
/// Guardian API request timeout in seconds
pub const GUARDIAN_API_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Consensus service API bind address
pub const CONSENSUS_API_BIND_ADDR: &str = "0.0.0.0";
/// Consensus service API port
pub const CONSENSUS_API_PORT: u16 = 8444;

/// Default log level (trace, debug, info, warn, error)
pub const LOG_LEVEL: &str = "info";

/// OpenObserve endpoint for log push (None to disable remote logging)
/// Uses bare hostname resolved by dnsmasq forwarding to Docker DNS proxy
pub const OPENOBSERVE_ENDPOINT: Option<&str> = Some("http://openobserve:5080");
/// OpenObserve organization
pub const OPENOBSERVE_ORG: &str = "default";
/// OpenObserve stream name for logs
pub const OPENOBSERVE_STREAM: &str = "guardian_logs";
/// OpenObserve credentials
pub const OPENOBSERVE_USER: &str = "root@example.com";
/// OpenObserve password
pub const OPENOBSERVE_PASSWORD: &str = "Complexpass#123";
/// Log push batch size (number of log entries before pushing)
pub const LOG_PUSH_BATCH_SIZE: usize = 50;
/// Log push interval in seconds
pub const LOG_PUSH_INTERVAL_SECS: u64 = 5;

/// Metrics endpoint path
pub const METRICS_ENDPOINT_PATH: &str = "/metrics";
/// Metrics collection interval in seconds
pub const METRICS_COLLECTION_INTERVAL_SECS: u64 = 15;
/// Enable metrics histogram buckets for latency measurements
pub const METRICS_ENABLE_HISTOGRAMS: bool = true;

/// Log file path for local file-based logging
pub const LOG_FILE_PATH: &str = "/var/log/guardian.log";
/// Maximum log file size in megabytes before truncation
pub const MAX_LOG_FILE_SIZE_MB: u64 = 12;

// ========== Gossip Protocol Configuration ==========

/// Interval for pulling peer lists from random trusted peers (seconds)
pub const GOSSIP_PULL_INTERVAL_SECS: u64 = 300; // 5 minutes
/// Maximum peers to return in peer list response
pub const GOSSIP_MAX_PEERS_PER_MESSAGE: usize = 50;
/// Maximum age of advertisements to accept (hours)
pub const GOSSIP_MAX_ADVERTISEMENT_AGE_HOURS: u64 = 24;

// ========== Peer Cache Configuration ==========

/// Path to peer metadata cache file
pub const PEER_CACHE_PATH: &str = "/var/lib/guardian/peer_cache.bin";

/// Interval for persisting peer cache to disk (seconds)
pub const PEER_CACHE_PERSIST_INTERVAL_SECS: u64 = 3600; // 1 hour

// ========== RAFT Storage Configuration ==========

/// Path to RAFT persistent storage directory
pub const RAFT_STORAGE_PATH: &str = "/var/lib/guardian/raft";

// ========== Instance Identity Configuration ==========

/// Path to instance_id file (TDX instances must generate at boot).
pub const INSTANCE_ID_PATH: &str = "/run/guardian/instance_id";

// ========== Observability API Configuration ==========

/// Port for observability API (separate from main API for isolation)
pub const OBSERVABILITY_API_PORT: u16 = 8445;

/// Number of worker threads for observability API (limited to prevent DoS impact)
pub const OBSERVABILITY_API_WORKERS: usize = 2;

/// Request timeout for observability API (seconds)
pub const OBSERVABILITY_API_TIMEOUT_SECS: u64 = 5;

// ========== Certificate Configuration ==========

/// Reserved namespace name for the Guardian cluster itself
pub const GUARDIAN_CERTIFICATE_NAMESPACE: &str = "guardian";
