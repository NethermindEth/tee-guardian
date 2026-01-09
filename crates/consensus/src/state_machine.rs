// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Guardian cluster state types.
//!
//! Shared data structures used by the RAFT storage implementation.

use std::collections::BTreeMap;

use openraft::{LogId, SnapshotMeta, StoredMembership};
use serde::{Deserialize, Serialize};

use crate::NodeId;

/// Request to modify cluster state (replicated via RAFT log).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterRequest {
    /// Client identifier
    pub client: String,
    /// Request serial number
    pub serial: u64,
    /// Status string
    pub status: String,
}

/// Response from cluster state modification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClusterResponse(pub Option<String>);

/// Snapshot of stored state for crash recovery.
#[derive(Debug)]
pub struct StoredSnapshot {
    /// Snapshot metadata (last log ID, membership)
    pub meta: SnapshotMeta<NodeId, openraft::BasicNode>,
    /// Serialized snapshot data
    pub data: Vec<u8>,
}

/// State machine data persisted to disk.
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct StateMachineData {
    /// Last applied log ID
    pub last_applied_log: Option<LogId<NodeId>>,
    /// Current cluster membership
    pub last_membership: StoredMembership<NodeId, openraft::BasicNode>,
    /// Application data (key-value store)
    pub data: BTreeMap<String, String>,
}
