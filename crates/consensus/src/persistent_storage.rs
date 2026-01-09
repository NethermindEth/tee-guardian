// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! File-backed persistent storage for RAFT consensus.
//!
//! # Design
//!
//! This module provides crash-safe persistent storage for RAFT state.
//! The key design principles are:
//!
//! 1. **Atomic writes**: All writes use temp file + rename pattern
//! 2. **Fsync discipline**: Vote and committed entries are fsynced before returning
//! 3. **Simple file layout**: One file per data type, easy to inspect and debug
//!
//! # File Layout
//!
//! ```text
//! /var/lib/guardian/raft/
//! ├── vote.bin           # Current vote (term, node_id, committed)
//! ├── committed.bin      # Last committed log ID
//! ├── logs/              # Log entries directory
//! │   ├── 00000001.bin   # Log entry at index 1
//! │   ├── 00000002.bin   # Log entry at index 2
//! │   └── ...
//! ├── state_machine.bin  # State machine data
//! └── snapshot/          # Snapshot directory
//!     ├── meta.bin       # Snapshot metadata
//!     └── data.bin       # Snapshot data
//! ```
//!
//! # Durability Guarantees
//!
//! - `save_vote()`: Fsyncs before returning (RAFT correctness requirement)
//! - `append()`: Writes are readable immediately, fsync called via callback
//! - `save_committed()`: Fsyncs before returning
//! - State machine: Fsynced on snapshot creation
//!
//! # Recovery
//!
//! On startup:
//! 1. Load vote from `vote.bin`
//! 2. Load committed log ID from `committed.bin`  
//! 3. Scan `logs/` directory for existing entries
//! 4. Load state machine from `state_machine.bin` or latest snapshot

use crate::state_machine::{ClusterResponse, StateMachineData, StoredSnapshot};
use crate::types::TypeConfig;
use crate::NodeId;
use openraft::storage::{LogFlushed, LogState, RaftLogStorage, RaftStateMachine, Snapshot};
use openraft::{
    Entry, EntryPayload, LogId, RaftLogId, RaftLogReader, RaftSnapshotBuilder, RaftTypeConfig, SnapshotMeta,
    StorageError, StorageIOError, StoredMembership, Vote,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Write};
use std::ops::RangeBounds;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info};

/// Directory names within the storage root
const LOGS_DIR: &str = "logs";
const SNAPSHOT_DIR: &str = "snapshot";

/// File names
const VOTE_FILE: &str = "vote.bin";
const COMMITTED_FILE: &str = "committed.bin";
const STATE_MACHINE_FILE: &str = "state_machine.bin";
const SNAPSHOT_META_FILE: &str = "meta.bin";
const SNAPSHOT_DATA_FILE: &str = "data.bin";

/// Persisted vote state.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedVote {
    vote: Vote<NodeId>,
}

/// Persisted committed log ID
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCommitted {
    log_id: Option<LogId<NodeId>>,
}

/// Persisted log entry (includes index for verification)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedLogEntry {
    index: u64,
    entry: Entry<TypeConfig>,
}

/// Persisted snapshot metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSnapshotMeta {
    meta: SnapshotMeta<NodeId, openraft::BasicNode>,
}

/// Atomically write data to a file using temp + rename pattern.
///
/// This ensures that readers either see the old file or the new file,
/// never a partially written file.
fn atomic_write(path: &Path, data: &[u8], fsync: bool) -> io::Result<()> {
    let parent =
        path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Path has no parent directory"))?;

    // Ensure parent directory exists
    fs::create_dir_all(parent)?;

    // Write to temp file
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(data)?;

    if fsync {
        file.sync_all()?;
    }

    // Atomic rename
    fs::rename(&temp_path, path)?;

    // Fsync parent directory to ensure rename is durable
    if fsync {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(())
}

/// Read entire file contents
fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    Ok(data)
}

/// Format log entry filename: 8 hex digits for index
fn log_entry_filename(index: u64) -> String {
    format!("{:08x}.bin", index)
}

/// Parse log index from filename
fn parse_log_index(filename: &str) -> Option<u64> {
    let stem = filename.strip_suffix(".bin")?;
    u64::from_str_radix(stem, 16).ok()
}

/// File-backed persistent storage for RAFT consensus.
///
/// This store persists all RAFT state to disk for crash recovery.
/// It implements both `RaftLogStorage` and `RaftStateMachine`.
#[derive(Clone)]
pub struct PersistentGuardianStore {
    /// Root directory for all storage
    root: PathBuf,
    /// In-memory log cache for fast reads
    log: Arc<Mutex<LogStoreInner>>,
    /// State machine
    state_machine: Arc<StateMachineStore>,
}

/// Inner log store state (cached in memory, persisted to disk)
#[derive(Debug, Default)]
struct LogStoreInner {
    /// Last purged log ID (entries before this have been deleted)
    last_purged_log_id: Option<LogId<NodeId>>,
    /// In-memory log cache: index -> entry
    log: BTreeMap<u64, Entry<TypeConfig>>,
    /// Last committed log ID
    committed: Option<LogId<NodeId>>,
    /// Current vote
    vote: Option<Vote<NodeId>>,
}

/// State machine store with snapshot support
#[derive(Debug, Default)]
pub struct StateMachineStore {
    /// State machine data
    pub state_machine: RwLock<StateMachineData>,
    /// Snapshot counter for unique IDs
    snapshot_idx: AtomicU64,
    /// Current snapshot (if any)
    current_snapshot: RwLock<Option<StoredSnapshot>>,
    /// Storage root path
    root: PathBuf,
}

impl PersistentGuardianStore {
    /// Create a new persistent store at the given path.
    ///
    /// If the directory contains existing state, it will be loaded.
    /// Otherwise, a new empty store is created.
    pub async fn new(root: PathBuf) -> io::Result<Self> {
        // Create directory structure
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join(LOGS_DIR))?;
        fs::create_dir_all(root.join(SNAPSHOT_DIR))?;

        let mut store = Self {
            root: root.clone(),
            log: Arc::new(Mutex::new(LogStoreInner::default())),
            state_machine: Arc::new(StateMachineStore {
                state_machine: RwLock::new(StateMachineData::default()),
                snapshot_idx: AtomicU64::new(0),
                current_snapshot: RwLock::new(None),
                root: root.clone(),
            }),
        };

        // Load existing state
        store.load_state().await?;

        info!(path = %root.display(), "Persistent RAFT storage initialized");
        Ok(store)
    }

    /// Load all persisted state from disk.
    async fn load_state(&mut self) -> io::Result<()> {
        let mut log_inner = self.log.lock().await;

        // Load vote
        let vote_path = self.root.join(VOTE_FILE);
        if vote_path.exists() {
            let data = read_file(&vote_path)?;
            let persisted: PersistedVote = bincode::deserialize(&data).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Failed to deserialize vote: {}", e))
            })?;
            log_inner.vote = Some(persisted.vote);
            debug!("Loaded vote from disk");
        }

        // Load committed
        let committed_path = self.root.join(COMMITTED_FILE);
        if committed_path.exists() {
            let data = read_file(&committed_path)?;
            let persisted: PersistedCommitted = bincode::deserialize(&data).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Failed to deserialize committed: {}", e))
            })?;
            log_inner.committed = persisted.log_id;
            debug!("Loaded committed log ID from disk");
        }

        // Load log entries
        let logs_dir = self.root.join(LOGS_DIR);
        if logs_dir.exists() {
            let mut entries: Vec<(u64, Entry<TypeConfig>)> = Vec::new();

            for entry in fs::read_dir(&logs_dir)? {
                let entry = entry?;
                let filename = entry.file_name();
                let filename_str = filename.to_string_lossy();

                if let Some(index) = parse_log_index(&filename_str) {
                    let data = read_file(&entry.path())?;
                    let persisted: PersistedLogEntry = bincode::deserialize(&data).map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Failed to deserialize log entry {}: {}", index, e),
                        )
                    })?;

                    if persisted.index != index {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Log entry index mismatch: filename says {}, content says {}",
                                index, persisted.index
                            ),
                        ));
                    }

                    entries.push((index, persisted.entry));
                }
            }

            // Sort by index and insert into log
            entries.sort_by_key(|(idx, _)| *idx);
            for (idx, entry) in entries {
                log_inner.log.insert(idx, entry);
            }

            if !log_inner.log.is_empty() {
                info!(
                    count = log_inner.log.len(),
                    first = log_inner.log.keys().next(),
                    last = log_inner.log.keys().last(),
                    "Loaded log entries from disk"
                );
            }
        }

        // Load state machine
        let sm_path = self.root.join(STATE_MACHINE_FILE);
        if sm_path.exists() {
            let data = read_file(&sm_path)?;
            let sm_data: StateMachineData = bincode::deserialize(&data).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Failed to deserialize state machine: {}", e))
            })?;
            *self.state_machine.state_machine.write().await = sm_data;
            debug!("Loaded state machine from disk");
        }

        // Load snapshot
        let snapshot_meta_path = self.root.join(SNAPSHOT_DIR).join(SNAPSHOT_META_FILE);
        let snapshot_data_path = self.root.join(SNAPSHOT_DIR).join(SNAPSHOT_DATA_FILE);
        if snapshot_meta_path.exists() && snapshot_data_path.exists() {
            let meta_data = read_file(&snapshot_meta_path)?;
            let persisted_meta: PersistedSnapshotMeta = bincode::deserialize(&meta_data).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Failed to deserialize snapshot meta: {}", e))
            })?;
            let snapshot_data = read_file(&snapshot_data_path)?;

            *self.state_machine.current_snapshot.write().await =
                Some(StoredSnapshot { meta: persisted_meta.meta, data: snapshot_data });
            debug!("Loaded snapshot from disk");
        }

        Ok(())
    }

    /// Get the logs directory path
    fn logs_dir(&self) -> PathBuf {
        self.root.join(LOGS_DIR)
    }

    /// Get the path for a log entry
    fn log_entry_path(&self, index: u64) -> PathBuf {
        self.logs_dir().join(log_entry_filename(index))
    }

    /// Persist vote to disk (with fsync)
    async fn persist_vote(&self, vote: &Vote<NodeId>) -> io::Result<()> {
        let persisted = PersistedVote { vote: vote.clone() };
        let data =
            bincode::serialize(&persisted).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        atomic_write(&self.root.join(VOTE_FILE), &data, true)
    }

    /// Persist committed log ID to disk (with fsync)
    async fn persist_committed(&self, committed: Option<LogId<NodeId>>) -> io::Result<()> {
        let persisted = PersistedCommitted { log_id: committed };
        let data =
            bincode::serialize(&persisted).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        atomic_write(&self.root.join(COMMITTED_FILE), &data, true)
    }

    /// Persist a log entry to disk (without fsync - callback will handle durability)
    async fn persist_log_entry(&self, entry: &Entry<TypeConfig>) -> io::Result<()> {
        let index = entry.get_log_id().index;
        let persisted = PersistedLogEntry { index, entry: entry.clone() };
        let data =
            bincode::serialize(&persisted).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        atomic_write(&self.log_entry_path(index), &data, false)
    }

    /// Delete a log entry from disk
    async fn delete_log_entry(&self, index: u64) -> io::Result<()> {
        let path = self.log_entry_path(index);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Fsync all log entries in a range
    async fn fsync_log_entries(&self) -> io::Result<()> {
        // Fsync the logs directory to ensure all entries are durable
        if let Ok(dir) = File::open(self.logs_dir()) {
            dir.sync_all()?;
        }
        Ok(())
    }

    /// Append entries directly (for testing only).
    ///
    /// This bypasses the callback mechanism and synchronously writes entries.
    #[cfg(test)]
    pub async fn append_entries_for_test(
        &mut self,
        entries: impl IntoIterator<Item = Entry<TypeConfig>>,
    ) -> io::Result<()> {
        let mut log_inner = self.log.lock().await;

        for entry in entries {
            self.persist_log_entry(&entry).await?;
            log_inner.log.insert(entry.get_log_id().index, entry);
        }

        drop(log_inner);
        self.fsync_log_entries().await
    }
}
impl RaftLogReader<TypeConfig> for PersistentGuardianStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<NodeId>> {
        let log_inner = self.log.lock().await;
        Ok(log_inner.log.range(range).map(|(_, val)| val.clone()).collect())
    }
}

impl RaftLogStorage<TypeConfig> for PersistentGuardianStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<NodeId>> {
        let log_inner = self.log.lock().await;
        let last = log_inner.log.iter().next_back().map(|(_, ent)| ent.get_log_id().clone());
        let last_purged = log_inner.last_purged_log_id.clone();
        let last = last.or(last_purged.clone());

        Ok(LogState { last_purged_log_id: last_purged, last_log_id: last })
    }

    async fn save_committed(&mut self, committed: Option<LogId<NodeId>>) -> Result<(), StorageError<NodeId>> {
        // Persist to disk first
        self.persist_committed(committed.clone()).await.map_err(|e| StorageIOError::write(&e))?;

        // Update in-memory cache
        self.log.lock().await.committed = committed;
        Ok(())
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<NodeId>>, StorageError<NodeId>> {
        Ok(self.log.lock().await.committed.clone())
    }

    async fn save_vote(&mut self, vote: &Vote<NodeId>) -> Result<(), StorageError<NodeId>> {
        // Persist to disk first (with fsync - RAFT correctness requirement)
        self.persist_vote(vote).await.map_err(|e| StorageIOError::write(&e))?;

        // Update in-memory cache
        self.log.lock().await.vote = Some(vote.clone());
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<NodeId>>, StorageError<NodeId>> {
        Ok(self.log.lock().await.vote.clone())
    }

    async fn append<I>(&mut self, entries: I, callback: LogFlushed<TypeConfig>) -> Result<(), StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>>,
    {
        let mut log_inner = self.log.lock().await;

        // Write entries to disk and memory
        for entry in entries {
            // Persist to disk (without fsync)
            self.persist_log_entry(&entry).await.map_err(|e| StorageIOError::write(&e))?;

            // Add to in-memory cache
            log_inner.log.insert(entry.get_log_id().index, entry);
        }

        // Drop lock before fsync (which may take time)
        drop(log_inner);

        // Fsync and notify callback
        self.fsync_log_entries().await.map_err(|e| StorageIOError::write(&e))?;
        callback.log_io_completed(Ok(()));

        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let mut log_inner = self.log.lock().await;

        // Collect keys to remove
        let keys: Vec<u64> = log_inner.log.range(log_id.index..).map(|(k, _)| *k).collect();

        // Remove from memory and disk
        for key in keys {
            log_inner.log.remove(&key);
            self.delete_log_entry(key).await.map_err(|e| StorageIOError::write(&e))?;
        }

        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<NodeId>) -> Result<(), StorageError<NodeId>> {
        let mut log_inner = self.log.lock().await;

        assert!(log_inner.last_purged_log_id.as_ref() <= Some(&log_id));
        log_inner.last_purged_log_id = Some(log_id.clone());

        // Collect keys to remove
        let keys: Vec<u64> = log_inner.log.range(..=log_id.index).map(|(k, _)| *k).collect();

        // Remove from memory and disk
        for key in keys {
            log_inner.log.remove(&key);
            self.delete_log_entry(key).await.map_err(|e| StorageIOError::write(&e))?;
        }

        Ok(())
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }
}

impl RaftSnapshotBuilder<TypeConfig> for Arc<StateMachineStore> {
    async fn build_snapshot(&mut self) -> Result<Snapshot<TypeConfig>, StorageError<NodeId>> {
        let state_machine = self.state_machine.read().await;
        let data = serde_json::to_vec(&state_machine.data).map_err(|e| StorageIOError::read_state_machine(&e))?;

        let last_applied_log = state_machine.last_applied_log;
        let last_membership = state_machine.last_membership.clone();
        drop(state_machine);

        let snapshot_idx = self.snapshot_idx.fetch_add(1, Ordering::Relaxed) + 1;
        let snapshot_id = if let Some(last) = last_applied_log {
            format!("{}-{}-{}", last.leader_id, last.index, snapshot_idx)
        } else {
            format!("--{}", snapshot_idx)
        };

        let meta = SnapshotMeta { last_log_id: last_applied_log, last_membership, snapshot_id };

        let snapshot = StoredSnapshot { meta: meta.clone(), data: data.clone() };

        // Persist snapshot to disk
        let snapshot_dir = self.root.join(SNAPSHOT_DIR);
        let meta_data =
            bincode::serialize(&PersistedSnapshotMeta { meta: meta.clone() }).map_err(|e| StorageIOError::write(&e))?;
        atomic_write(&snapshot_dir.join(SNAPSHOT_META_FILE), &meta_data, true)
            .map_err(|e| StorageIOError::write(&e))?;
        atomic_write(&snapshot_dir.join(SNAPSHOT_DATA_FILE), &data, true).map_err(|e| StorageIOError::write(&e))?;

        *self.current_snapshot.write().await = Some(snapshot);

        Ok(Snapshot { meta, snapshot: Box::new(Cursor::new(data)) })
    }
}

impl RaftStateMachine<TypeConfig> for Arc<StateMachineStore> {
    type SnapshotBuilder = Self;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, openraft::BasicNode>), StorageError<NodeId>> {
        let state_machine = self.state_machine.read().await;
        Ok((state_machine.last_applied_log, state_machine.last_membership.clone()))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<ClusterResponse>, StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
    {
        let mut res = Vec::new();
        let mut sm = self.state_machine.write().await;

        for entry in entries {
            sm.last_applied_log = Some(entry.log_id);

            match entry.payload {
                EntryPayload::Blank => res.push(ClusterResponse::default()),
                EntryPayload::Normal(_) => res.push(ClusterResponse::default()),
                EntryPayload::Membership(ref mem) => {
                    sm.last_membership = StoredMembership::new(Some(entry.log_id), mem.clone());
                    res.push(ClusterResponse::default())
                }
            }
        }

        // Persist state machine after applying entries
        drop(sm);
        let sm = self.state_machine.read().await;
        let data = bincode::serialize(&*sm).map_err(|e| StorageIOError::write(&e))?;
        drop(sm);

        atomic_write(&self.root.join(STATE_MACHINE_FILE), &data, true).map_err(|e| StorageIOError::write(&e))?;

        Ok(res)
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, StorageError<NodeId>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, openraft::BasicNode>,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<NodeId>> {
        let data = snapshot.into_inner();
        let updated_data: BTreeMap<String, String> =
            serde_json::from_slice(&data).map_err(|e| StorageIOError::read_snapshot(Some(meta.signature()), &e))?;

        let updated_state_machine = StateMachineData {
            last_applied_log: meta.last_log_id,
            last_membership: meta.last_membership.clone(),
            data: updated_data,
        };

        *self.state_machine.write().await = updated_state_machine;

        let new_snapshot = StoredSnapshot { meta: meta.clone(), data: data.clone() };

        // Persist to disk
        let snapshot_dir = self.root.join(SNAPSHOT_DIR);
        let meta_data =
            bincode::serialize(&PersistedSnapshotMeta { meta: meta.clone() }).map_err(|e| StorageIOError::write(&e))?;
        atomic_write(&snapshot_dir.join(SNAPSHOT_META_FILE), &meta_data, true)
            .map_err(|e| StorageIOError::write(&e))?;
        atomic_write(&snapshot_dir.join(SNAPSHOT_DATA_FILE), &data, true).map_err(|e| StorageIOError::write(&e))?;

        // Persist state machine
        let sm = self.state_machine.read().await;
        let sm_data = bincode::serialize(&*sm).map_err(|e| StorageIOError::write(&e))?;
        drop(sm);
        atomic_write(&self.root.join(STATE_MACHINE_FILE), &sm_data, true).map_err(|e| StorageIOError::write(&e))?;

        *self.current_snapshot.write().await = Some(new_snapshot);

        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<NodeId>> {
        match &*self.current_snapshot.read().await {
            Some(snapshot) => {
                let data = snapshot.data.clone();
                Ok(Some(Snapshot { meta: snapshot.meta.clone(), snapshot: Box::new(Cursor::new(data)) }))
            }
            None => Ok(None),
        }
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }
}

impl RaftStateMachine<TypeConfig> for PersistentGuardianStore {
    type SnapshotBuilder = Arc<StateMachineStore>;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<NodeId>>, StoredMembership<NodeId, openraft::BasicNode>), StorageError<NodeId>> {
        Arc::clone(&self.state_machine).applied_state().await
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<ClusterResponse>, StorageError<NodeId>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        Arc::clone(&self.state_machine).apply(entries).await
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<<TypeConfig as RaftTypeConfig>::SnapshotData>, StorageError<NodeId>> {
        Arc::clone(&self.state_machine).begin_receiving_snapshot().await
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<NodeId, openraft::BasicNode>,
        snapshot: Box<<TypeConfig as RaftTypeConfig>::SnapshotData>,
    ) -> Result<(), StorageError<NodeId>> {
        Arc::clone(&self.state_machine).install_snapshot(meta, snapshot).await
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot<TypeConfig>>, StorageError<NodeId>> {
        Arc::clone(&self.state_machine).get_current_snapshot().await
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        Arc::clone(&self.state_machine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openraft::{CommittedLeaderId, Membership};
    use tempfile::tempdir;

    /// Create a blank log entry for testing
    fn blank_entry(term: u64, index: u64) -> Entry<TypeConfig> {
        Entry { log_id: LogId::new(CommittedLeaderId::new(term, 1), index), payload: EntryPayload::Blank }
    }

    /// Create a membership log entry for testing
    fn membership_entry(term: u64, index: u64, members: Vec<NodeId>) -> Entry<TypeConfig> {
        let membership = Membership::new(vec![members.into_iter().collect()], None);
        Entry {
            log_id: LogId::new(CommittedLeaderId::new(term, 1), index),
            payload: EntryPayload::Membership(membership),
        }
    }

    #[tokio::test]
    async fn test_new_store() {
        let dir = tempdir().unwrap();
        let store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Verify directories were created
        assert!(dir.path().join(LOGS_DIR).exists());
        assert!(dir.path().join(SNAPSHOT_DIR).exists());

        // Verify empty state
        assert!(store.log.lock().await.log.is_empty());
    }

    #[tokio::test]
    async fn test_vote_persistence() {
        let dir = tempdir().unwrap();
        let mut store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Save vote
        let vote = Vote::new_committed(5, 1);
        store.save_vote(&vote).await.unwrap();

        // Create new store and verify vote was loaded
        let mut store2 = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();
        let loaded_vote = store2.read_vote().await.unwrap();
        assert!(loaded_vote.is_some());
    }

    #[tokio::test]
    async fn test_committed_persistence() {
        let dir = tempdir().unwrap();
        let mut store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Save committed
        let log_id = LogId::new(CommittedLeaderId::new(1, 1), 42);
        store.save_committed(Some(log_id.clone())).await.unwrap();

        // Create new store and verify committed was loaded
        let mut store2 = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();
        let loaded = store2.read_committed().await.unwrap();
        assert_eq!(loaded, Some(log_id));
    }

    #[tokio::test]
    async fn test_log_entry_persistence() {
        let dir = tempdir().unwrap();
        let mut store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Append entries using test helper
        let entries = vec![blank_entry(1, 1), blank_entry(1, 2), blank_entry(1, 3)];
        store.append_entries_for_test(entries).await.unwrap();

        // Create new store and verify entries were loaded
        let mut store2 = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();
        let loaded = store2.try_get_log_entries(1..=3).await.unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].get_log_id().index, 1);
        assert_eq!(loaded[1].get_log_id().index, 2);
        assert_eq!(loaded[2].get_log_id().index, 3);
    }

    #[tokio::test]
    async fn test_truncate() {
        let dir = tempdir().unwrap();
        let mut store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Append entries using test helper
        let entries = vec![blank_entry(1, 1), blank_entry(1, 2), blank_entry(1, 3)];
        store.append_entries_for_test(entries).await.unwrap();

        // Truncate from index 2
        store.truncate(LogId::new(CommittedLeaderId::new(1, 1), 2)).await.unwrap();

        // Verify only entry 1 remains
        let loaded = store.try_get_log_entries(1..=3).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].get_log_id().index, 1);

        // Verify files were deleted
        assert!(!dir.path().join(LOGS_DIR).join("00000002.bin").exists());
        assert!(!dir.path().join(LOGS_DIR).join("00000003.bin").exists());
    }

    #[tokio::test]
    async fn test_purge() {
        let dir = tempdir().unwrap();
        let mut store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Append entries using test helper
        let entries = vec![blank_entry(1, 1), blank_entry(1, 2), blank_entry(1, 3)];
        store.append_entries_for_test(entries).await.unwrap();

        // Purge up to index 2
        store.purge(LogId::new(CommittedLeaderId::new(1, 1), 2)).await.unwrap();

        // Verify only entry 3 remains
        let loaded = store.try_get_log_entries(1..=3).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].get_log_id().index, 3);
    }

    #[tokio::test]
    async fn test_state_machine_persistence() {
        let dir = tempdir().unwrap();
        let mut store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Apply membership entry
        let entries = vec![membership_entry(1, 1, vec![1, 2, 3])];
        store.apply(entries).await.unwrap();

        // Create new store and verify state machine was loaded
        let mut store2 = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();
        let (last_applied, membership) = store2.applied_state().await.unwrap();
        assert!(last_applied.is_some());
        assert_eq!(last_applied.unwrap().index, 1);
        assert!(membership.voter_ids().count() > 0);
    }

    #[tokio::test]
    async fn test_snapshot_persistence() {
        let dir = tempdir().unwrap();
        let mut store = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();

        // Apply some entries
        let entries = vec![blank_entry(1, 1)];
        store.apply(entries).await.unwrap();

        // Build snapshot
        let mut builder = store.get_snapshot_builder().await;
        let snapshot = builder.build_snapshot().await.unwrap();
        assert!(snapshot.meta.last_log_id.is_some());

        // Create new store and verify snapshot was loaded
        let mut store2 = PersistentGuardianStore::new(dir.path().to_path_buf()).await.unwrap();
        let loaded_snapshot = store2.get_current_snapshot().await.unwrap();
        assert!(loaded_snapshot.is_some());
    }

    #[test]
    fn test_log_entry_filename() {
        assert_eq!(log_entry_filename(1), "00000001.bin");
        assert_eq!(log_entry_filename(255), "000000ff.bin");
        assert_eq!(log_entry_filename(0xDEADBEEF), "deadbeef.bin");
    }

    #[test]
    fn test_parse_log_index() {
        assert_eq!(parse_log_index("00000001.bin"), Some(1));
        assert_eq!(parse_log_index("000000ff.bin"), Some(255));
        assert_eq!(parse_log_index("deadbeef.bin"), Some(0xDEADBEEF));
        assert_eq!(parse_log_index("invalid.bin"), None);
        assert_eq!(parse_log_index("00000001.txt"), None);
    }
}
