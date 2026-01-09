// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! RAFT persistent storage.
//!
//! File-backed storage for crash-safe RAFT consensus.
//!
//! # File Layout
//!
//! ```text
//! /var/lib/guardian/raft/
//! ├── vote.bin           # Current vote (term, node_id, committed)
//! ├── committed.bin      # Last committed log ID
//! ├── logs/              # Log entries directory
//! │   ├── 00000001.bin   # Log entry at index 1
//! │   └── ...
//! ├── state_machine.bin  # State machine data
//! └── snapshot/          # Snapshot directory
//!     ├── meta.bin       # Snapshot metadata
//!     └── data.bin       # Snapshot data
//! ```

use std::path::Path;

pub use crate::persistent_storage::PersistentGuardianStore;
pub use crate::types::TypeConfig;

/// The RAFT storage type.
pub type Store = PersistentGuardianStore;

/// Create a new persistent store at the given path.
///
/// State is persisted to disk and survives restarts.
pub async fn new_store(path: impl AsRef<Path>) -> std::io::Result<Store> {
    PersistentGuardianStore::new(path.as_ref().to_path_buf()).await
}
