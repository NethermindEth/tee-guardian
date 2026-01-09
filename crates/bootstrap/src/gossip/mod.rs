// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Peer Gossip Protocol
//!
//! Implements peer discovery via gossip with TEE-backed authentication.
//! Advertisements include a TDX/SEV-SNP quote that binds the advertised
//! addresses to the TEE instance.
//!
//! # Security Model
//!
//! Each advertisement contains TEE evidence with:
//! - `REPORTDATA[0:32]` = `SHA256(raft_address || api_address)` - binds addresses
//! - `REPORTDATA[32:64]` = `instance_id` - binds identity
//!
//! This ensures:
//! 1. Only real TEE instances can create valid advertisements
//! 2. Advertisements cannot be replayed with different addresses
//! 3. The certificate chain traces back to Intel/AMD root of trust
//!
//! # Trust Flow
//!
//! ```text
//! POST /advertise
//!    │
//!    ▼
//! Parse quote (no crypto) ──► Extract instance_id
//!    │
//!    ▼
//! Verify DCAP/VCEK signature ──► Confirms TEE authenticity
//!    │
//!    ▼
//! Verify REPORTDATA binding ──► Confirms address ownership
//!    │
//!    ▼
//! Add to cache (trusted discovery)
//! ```
//!
//! # Module Structure
//!
//! - [`types`] - Advertisement and response types
//! - [`cache`] - Thread-safe cache for verified advertisements
//! - [`config`] - Gossip protocol configuration
//! - [`manager`] - Main gossip manager with push/pull logic
//! - [`evidence`] - TEE evidence generation

mod cache;
mod config;
mod evidence;
mod manager;
mod types;

// Re-export public API
pub use cache::GossipedPeerCache;
pub use config::GossipConfig;
pub use evidence::generate_gossip_evidence;
pub use manager::PeerGossipManager;
pub use types::{AdvertiseResponse, PeerAdvertisement, PeerListResponse};
