// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Guardian - TEE-based Key Management Service
//!
//! Main service that coordinates all Guardian functionality:
//! - TDX attestation generation and verification
//! - RAFT consensus for distributed key management
//! - HTTP API for all operations

pub mod api_server;
pub mod observability_api;

// Re-export for convenience
pub use api_server::{start_guardian_api_server, ApiState, SetLogContextRequest, SetLogContextResponse};
pub use common::HealthResponse;
pub use observability_api::start_observability_api;
