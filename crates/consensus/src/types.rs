// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Type definitions for RAFT consensus

use serde::{Deserialize, Serialize};
// Note: Cursor is required by openraft::declare_raft_types! macro expansion
use std::io::Cursor;
use std::net::SocketAddr;

// Re-export NodeId from peer_registry (single source of truth)
pub use peer_registry::types::NodeId;

use crate::state_machine::{ClusterRequest, ClusterResponse};

openraft::declare_raft_types!(
    pub TypeConfig:
        D = ClusterRequest,
        R = ClusterResponse,
        Node = openraft::BasicNode,
);

pub type Raft = openraft::Raft<TypeConfig>;

pub mod typ {
    use crate::NodeId;
    use openraft::BasicNode;

    pub type RaftError<E = openraft::error::Infallible> = openraft::error::RaftError<NodeId, E>;
    pub type RPCError<E = openraft::error::Infallible> = openraft::error::RPCError<NodeId, BasicNode, RaftError<E>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    pub node_id: NodeId,
    pub raft_address: SocketAddr,
    pub api_address: SocketAddr,
    pub measurement_registry_url: String,
    pub measurement_namespace: String,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            node_id: 1,
            raft_address: "0.0.0.0:8444".parse().unwrap(),
            api_address: "0.0.0.0:8443".parse().unwrap(),
            measurement_registry_url: common::config::MEASUREMENT_REGISTRY_URL.to_string(),
            measurement_namespace: "guardian-cluster".to_string(),
        }
    }
}

/// Node state for health checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeState {
    /// Node lifecycle state.
    pub status: common::NodeStatus,
    /// Current RAFT term
    pub term: u64,
}
