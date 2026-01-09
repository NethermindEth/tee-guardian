// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Gossip protocol configuration.

use common::{GOSSIP_MAX_ADVERTISEMENT_AGE_HOURS, GOSSIP_MAX_PEERS_PER_MESSAGE, GOSSIP_PULL_INTERVAL_SECS};

/// Configuration for gossip protocol.
#[derive(Debug, Clone)]
pub struct GossipConfig {
    /// Interval between pull requests to random trusted peers (seconds).
    pub pull_interval_secs: u64,

    /// Maximum peers to include in a peer list response.
    pub max_peers_per_message: usize,

    /// Maximum age of advertisements to accept (hours).
    pub max_advertisement_age_hours: u64,
}

impl Default for GossipConfig {
    fn default() -> Self {
        Self {
            pull_interval_secs: GOSSIP_PULL_INTERVAL_SECS,
            max_peers_per_message: GOSSIP_MAX_PEERS_PER_MESSAGE,
            max_advertisement_age_hours: GOSSIP_MAX_ADVERTISEMENT_AGE_HOURS,
        }
    }
}
