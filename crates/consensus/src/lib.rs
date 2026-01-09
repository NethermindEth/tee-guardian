// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Pure RAFT consensus implementation.
//!
//! # Architecture
//!
//! This crate implements OpenRaft-based consensus with trust-gated networking.
//! It is intentionally attestation-agnostic - all trust/attestation logic has been
//! moved to the `bootstrap` and `peer_registry` crates.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       Consensus Crate                           │
//! │                                                                  │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐ │
//! │  │  RaftNode   │  │   Router    │  │ TrustedPeerRegistry     │ │
//! │  │  (OpenRaft) │──│ (RPC gate)  │──│ (from peer_registry)    │ │
//! │  └─────────────┘  └─────────────┘  └─────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The RAFT layer relies on `Router` to block RPC to untrusted peers via the
//! `TrustedPeerRegistry`. Untrusted peers appear as "unreachable" to OpenRaft.
//!
//! See `docs/bootstrap_sequence.md` for the full cluster formation protocol.

pub mod coordinator;
pub mod error;
pub mod metrics;
pub mod network;
pub mod persistent_storage;
pub mod raft_api_server;
pub mod raft_node;
pub mod state_machine;
pub mod storage;
pub mod types;

// Re-exports
pub use coordinator::RaftCoordinator;
pub use error::{ConsensusError, ConsensusResult};
pub use network::Router;
pub use persistent_storage::PersistentGuardianStore;
pub use raft_api_server::{start_raft_api_server, RaftApiState};
pub use raft_node::RaftNode;
pub use storage::{new_store, Store};
pub use types::{ClusterConfig, NodeId};
