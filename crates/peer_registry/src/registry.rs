// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! PeerRegistry - Central TEE state management with event broadcasting.
//!
//! The [`PeerRegistry`] tracks TEE attestation state and broadcasts events
//! to subscribers when state changes occur.
//!
//! # Event-Driven Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        PeerRegistry                              │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  pending_challenges: HashMap<InstanceId, TeeRecord>             │
//! │  verified_tees: HashMap<InstanceId, TeeRecord>                  │
//! │  event_tx: broadcast::Sender<PeerRegistryEvent>                 │
//! └─────────────────────────────────────────────────────────────────┘
//!                               │
//!            ┌──────────────────┼──────────────────┐
//!            │                  │                  │
//!            ▼                  ▼                  ▼
//!     ┌────────────┐    ┌────────────┐    ┌────────────┐
//!     │  Guardian  │    │  K8s Kata  │    │  Builder   │
//!     │ Subscriber │    │ Subscriber │    │ Subscriber │
//!     │ ns=guardian│    │ ns=k8s-prod│    │ ns=builder │
//!     └────────────┘    └────────────┘    └────────────┘
//! ```

use crate::types::identity::InstanceId;
use crate::types::tee::{PeerRegistryEvent, TeeRecord, TeeState};
use kbs_types::Tee;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn};

/// Default challenge expiration time.
const DEFAULT_CHALLENGE_EXPIRY: Duration = Duration::from_secs(60);

/// Broadcast channel capacity for events.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Central registry for TEE attestation state.
///
/// Manages the two-phase authentication protocol:
/// 1. `register()` - Issue challenge, store pending record
/// 2. `verify()` - Validate challenge, transition to verified
///
/// State changes are broadcast to all subscribers.
#[derive(Clone)]
pub struct PeerRegistry {
    inner: Arc<PeerRegistryInner>,
}

struct PeerRegistryInner {
    /// TEEs with pending challenges (Phase 1 complete, awaiting Phase 2)
    pending: RwLock<HashMap<InstanceId, TeeRecord>>,
    /// Successfully verified TEEs
    verified: RwLock<HashMap<InstanceId, TeeRecord>>,
    /// Event broadcaster
    event_tx: broadcast::Sender<PeerRegistryEvent>,
    /// Challenge expiration duration
    challenge_expiry: Duration,
}

impl PeerRegistry {
    /// Create a new peer registry with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(DEFAULT_CHALLENGE_EXPIRY)
    }

    /// Create a new peer registry with custom challenge expiry.
    #[must_use]
    pub fn with_config(challenge_expiry: Duration) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(PeerRegistryInner {
                pending: RwLock::new(HashMap::new()),
                verified: RwLock::new(HashMap::new()),
                event_tx,
                challenge_expiry,
            }),
        }
    }

    /// Subscribe to registry events.
    ///
    /// Events include namespace information for filtering.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<PeerRegistryEvent> {
        self.inner.event_tx.subscribe()
    }

    /// Register a TEE and issue a challenge (Phase 1).
    ///
    /// # Arguments
    ///
    /// * `instance_id` - Unique instance identity from evidence (REPORTDATA[32:64] for TDX)
    /// * `namespace` - Namespace the TEE is registering for
    /// * `measurement_hash` - Pre-verified measurement hash from Phase 1
    /// * `tee` - TEE platform type (TDX, SEV-SNP, etc.)
    ///
    /// # Returns
    ///
    /// - `Ok((challenge, is_new))` - The challenge nonce and whether this is a new registration
    /// - `Err(true)` - Instance ID already exists in verified (collision)
    ///
    /// If the instance_id already has a pending challenge, the existing challenge is returned.
    pub async fn register(
        &self,
        instance_id: InstanceId,
        namespace: String,
        measurement_hash: String,
        tee: Tee,
    ) -> Result<([u8; 32], bool), bool> {
        // Check if already verified (collision - reject)
        {
            let verified = self.inner.verified.read().await;
            if verified.contains_key(&instance_id) {
                warn!(
                    instance_id = %instance_id,
                    namespace = %namespace,
                    "Registration rejected: instance already verified"
                );
                return Err(true);
            }
        }

        // Check if already pending (return existing challenge)
        {
            let pending = self.inner.pending.read().await;
            if let Some(record) = pending.get(&instance_id) {
                if let Some(challenge) = record.get_valid_challenge() {
                    debug!(
                        instance_id = %instance_id,
                        namespace = %namespace,
                        "Returning existing pending challenge"
                    );
                    return Ok((*challenge, false));
                }
                // Challenge expired, will be replaced below
            }
        }

        // Generate random challenge
        let challenge: [u8; 32] = rand::random();

        // Create pending record
        let record = TeeRecord::new_pending(
            instance_id,
            namespace.clone(),
            challenge,
            measurement_hash.clone(),
            tee,
            self.inner.challenge_expiry,
        );

        // Store pending challenge
        {
            let mut pending = self.inner.pending.write().await;
            pending.insert(instance_id, record);
        }

        info!(
            instance_id = %instance_id,
            namespace = %namespace,
            "TEE registered, challenge issued"
        );

        // Broadcast event
        let _ = self.inner.event_tx.send(PeerRegistryEvent::Registered {
            namespace,
            instance_id,
            challenge,
            measurement_hash,
        });

        Ok((challenge, true))
    }

    /// Get the pending challenge for an instance (if valid).
    ///
    /// Returns `None` if:
    /// - Instance ID not found in pending
    /// - Challenge has expired
    pub async fn get_pending_challenge(&self, instance_id: &InstanceId) -> Option<[u8; 32]> {
        let pending = self.inner.pending.read().await;
        pending.get(instance_id).and_then(|record| record.get_valid_challenge().copied())
    }

    /// Get the pending record for an instance.
    pub async fn get_pending_record(&self, instance_id: &InstanceId) -> Option<TeeRecord> {
        let pending = self.inner.pending.read().await;
        pending.get(instance_id).cloned()
    }

    /// Complete verification and transition to verified state (Phase 2).
    ///
    /// # Arguments
    ///
    /// * `instance_id` - Instance ID from Phase 1
    /// * `state` - Verified state with attestation details
    ///
    /// # Returns
    ///
    /// The complete TeeRecord if successful.
    pub async fn complete_verification(&self, instance_id: InstanceId, state: TeeState) -> Option<TeeRecord> {
        // Remove from pending
        let pending_record = {
            let mut pending = self.inner.pending.write().await;
            pending.remove(&instance_id)
        }?;

        // Create verified record
        let record = TeeRecord {
            instance_id,
            namespace: pending_record.namespace.clone(),
            state,
            ip_address: pending_record.ip_address,
            api_port: pending_record.api_port,
        };

        // Store in verified
        {
            let mut verified = self.inner.verified.write().await;
            verified.insert(instance_id, record.clone());
        }

        info!(
            instance_id = %instance_id,
            namespace = %record.namespace,
            "TEE verification complete"
        );

        // Broadcast event
        let _ = self.inner.event_tx.send(PeerRegistryEvent::Verified {
            namespace: record.namespace.clone(),
            instance_id,
            record: record.clone(),
        });

        Some(record)
    }

    /// Reject a TEE (failed verification).
    pub async fn reject(&self, instance_id: InstanceId, namespace: String, reason: String) {
        // Remove from pending if present
        {
            let mut pending = self.inner.pending.write().await;
            pending.remove(&instance_id);
        }

        warn!(
            instance_id = %instance_id,
            namespace = %namespace,
            reason = %reason,
            "TEE rejected"
        );

        // Broadcast event
        let _ = self.inner.event_tx.send(PeerRegistryEvent::Rejected { namespace, instance_id, reason });
    }

    /// Revoke trust from a verified TEE.
    pub async fn revoke(&self, instance_id: &InstanceId, reason: String) -> bool {
        // Remove from verified
        let record = {
            let mut verified = self.inner.verified.write().await;
            verified.remove(instance_id)
        };

        if let Some(record) = record {
            warn!(
                instance_id = %instance_id,
                namespace = %record.namespace,
                reason = %reason,
                "TEE trust revoked"
            );

            // Broadcast event
            let _ = self.inner.event_tx.send(PeerRegistryEvent::Revoked {
                namespace: record.namespace.clone(),
                instance_id: *instance_id,
                reason,
                record,
            });

            true
        } else {
            false
        }
    }

    /// Check if an instance is verified.
    pub async fn is_verified(&self, instance_id: &InstanceId) -> bool {
        let verified = self.inner.verified.read().await;
        verified.contains_key(instance_id)
    }

    /// Get a verified TEE record by instance ID.
    pub async fn get_verified(&self, instance_id: &InstanceId) -> Option<TeeRecord> {
        let verified = self.inner.verified.read().await;
        verified.get(instance_id).cloned()
    }

    /// Get all verified TEEs for a namespace.
    pub async fn get_verified_by_namespace(&self, namespace: &str) -> Vec<TeeRecord> {
        let verified = self.inner.verified.read().await;
        verified.values().filter(|r| r.namespace == namespace).cloned().collect()
    }

    /// Get count of verified TEEs.
    pub async fn verified_count(&self) -> usize {
        let verified = self.inner.verified.read().await;
        verified.len()
    }

    /// Get count of pending challenges.
    pub async fn pending_count(&self) -> usize {
        let pending = self.inner.pending.read().await;
        pending.len()
    }

    /// Clean up expired pending challenges.
    ///
    /// Call this periodically to prevent memory growth from abandoned registrations.
    pub async fn cleanup_expired(&self) -> usize {
        let mut pending = self.inner.pending.write().await;
        let before = pending.len();

        pending.retain(|instance_id, record| {
            if record.is_challenge_expired() {
                debug!(instance_id = %instance_id, "Removing expired pending challenge");
                false
            } else {
                true
            }
        });

        before - pending.len()
    }
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;

    #[tokio::test]
    async fn test_register_and_get_challenge() {
        let registry = PeerRegistry::new();
        let instance_id = InstanceId::from_bytes([0xAB; 32]);

        let (challenge, is_new) =
            registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await.unwrap();

        assert!(is_new);
        assert_eq!(challenge.len(), 32);

        let retrieved = registry.get_pending_challenge(&instance_id).await;
        assert_eq!(retrieved, Some(challenge));
    }

    #[tokio::test]
    async fn test_register_returns_existing_challenge() {
        let registry = PeerRegistry::new();
        let instance_id = InstanceId::from_bytes([0xAB; 32]);

        // First registration
        let (challenge1, is_new1) =
            registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await.unwrap();
        assert!(is_new1);

        // Second registration with same instance_id returns existing challenge
        let (challenge2, is_new2) =
            registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await.unwrap();
        assert!(!is_new2);
        assert_eq!(challenge1, challenge2);
    }

    #[tokio::test]
    async fn test_register_rejects_verified_instance() {
        let registry = PeerRegistry::new();
        let instance_id = InstanceId::from_bytes([0xAB; 32]);

        // Register and verify
        let (_, _) =
            registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await.unwrap();

        let state = TeeState::Verified {
            measurement_hash: "abc123".to_string(),
            tcb_status: "UpToDate".to_string(),
            tee: Tee::Tdx,
            verified_at: Utc::now(),
        };
        registry.complete_verification(instance_id, state).await;

        // Try to register again - should fail
        let result = registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_complete_verification() {
        let registry = PeerRegistry::new();
        let instance_id = InstanceId::from_bytes([0xAB; 32]);

        // Register
        let (_, _) =
            registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await.unwrap();

        // Complete verification
        let state = TeeState::Verified {
            measurement_hash: "abc123".to_string(),
            tcb_status: "UpToDate".to_string(),
            tee: Tee::Tdx,
            verified_at: Utc::now(),
        };

        let record = registry.complete_verification(instance_id, state).await;
        assert!(record.is_some());

        // Should be in verified, not pending
        assert!(registry.is_verified(&instance_id).await);
        assert!(registry.get_pending_challenge(&instance_id).await.is_none());
    }

    #[tokio::test]
    async fn test_revoke() {
        let registry = PeerRegistry::new();
        let instance_id = InstanceId::from_bytes([0xAB; 32]);

        // Register and verify
        let (_, _) =
            registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await.unwrap();
        let state = TeeState::Verified {
            measurement_hash: "abc123".to_string(),
            tcb_status: "UpToDate".to_string(),
            tee: Tee::Tdx,
            verified_at: Utc::now(),
        };
        registry.complete_verification(instance_id, state).await;

        assert!(registry.is_verified(&instance_id).await);

        // Revoke
        let revoked = registry.revoke(&instance_id, "test revocation".to_string()).await;
        assert!(revoked);
        assert!(!registry.is_verified(&instance_id).await);
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let registry = PeerRegistry::new();
        let mut events = registry.subscribe();
        let instance_id = InstanceId::from_bytes([0xAB; 32]);

        // Register should emit event
        registry.register(instance_id, "guardian".to_string(), "abc123".to_string(), Tee::Tdx).await.unwrap();

        let event = events.try_recv().unwrap();
        assert!(matches!(event, PeerRegistryEvent::Registered { .. }));
        assert_eq!(event.namespace(), "guardian");
    }

    #[tokio::test]
    async fn test_get_verified_by_namespace() {
        let registry = PeerRegistry::new();

        // Register TEEs to different namespaces with different instance IDs
        let instance1 = InstanceId::from_bytes([0x01; 32]);
        let instance2 = InstanceId::from_bytes([0x02; 32]);
        let instance3 = InstanceId::from_bytes([0x03; 32]);

        registry.register(instance1, "guardian".to_string(), "hash1".to_string(), Tee::Tdx).await.unwrap();
        registry.register(instance2, "guardian".to_string(), "hash2".to_string(), Tee::Tdx).await.unwrap();
        registry.register(instance3, "k8s-prod".to_string(), "hash3".to_string(), Tee::Tdx).await.unwrap();

        let verified_state = TeeState::Verified {
            measurement_hash: "hash".to_string(),
            tcb_status: "UpToDate".to_string(),
            tee: Tee::Tdx,
            verified_at: Utc::now(),
        };

        registry.complete_verification(instance1, verified_state.clone()).await;
        registry.complete_verification(instance2, verified_state.clone()).await;
        registry.complete_verification(instance3, verified_state.clone()).await;

        let guardian_tees = registry.get_verified_by_namespace("guardian").await;
        assert_eq!(guardian_tees.len(), 2);

        let k8s_tees = registry.get_verified_by_namespace("k8s-prod").await;
        assert_eq!(k8s_tees.len(), 1);
    }
}
