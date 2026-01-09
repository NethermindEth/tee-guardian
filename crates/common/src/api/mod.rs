// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Shared API types and error handling.
//!
//! This module contains types used across the Guardian HTTP API and internal
//! components like the RaftCoordinator.

pub mod error;
pub mod types;

pub use error::{ApiError, ErrorResponse};
pub use types::{HealthResponse, NodeStatus, MIN_PEERS_FOR_BOOTSTRAP};
