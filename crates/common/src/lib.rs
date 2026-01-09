// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Common utilities and configuration for TEE Guardian
//!
//! This crate contains shared configuration, logging, metrics, and API types.

#![warn(missing_docs)]
// Allow unsafe code only in the sensitive module for zeroization
#![allow(unsafe_code)]

pub mod api;
pub mod bytes;
pub mod config;
pub mod logging;
pub mod metrics;
pub mod network;
pub mod sensitive;
pub mod time;

// Re-export for convenience
pub use api::{ApiError, ErrorResponse, HealthResponse, NodeStatus, MIN_PEERS_FOR_BOOTSTRAP};
pub use bytes::{Bytes32, Bytes48, Bytes64};
pub use config::*;
pub use network::{get_node_ip_from_dhcp, NetworkError};
pub use time::{unix_timestamp_micros, unix_timestamp_secs};
