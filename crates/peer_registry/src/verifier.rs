// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Platform-agnostic TEE attestation verification.
//!
//! This module provides the two-phase attestation protocol using the
//! [`TdxQuote`](crate::backend::TdxQuote) and [`SnpReport`](crate::backend::SnpReport)
//! types for platform-specific verification.
//!
//! # Two-Phase Protocol
//!
//! ```text
//! Phase 1: POST /attestation/{ns}/register { evidence }
//!   → Verify signature, extract instance_id, check measurement in registry
//!   ← { challenge, expires_at }
//!
//! Phase 2: POST /attestation/{ns}/verify { evidence }
//!   → Extract instance_id, look up challenge, verify with nonce binding
//!   ← { verified: true, measurement_hash, tcb_status }
//! ```

use crate::backend::{detect_tee_type, SnpReport, TdxQuote};
use crate::measurement::MeasurementRegistryClient;
use crate::registry::PeerRegistry;
use crate::types::{InstanceId, RegistrationResponse, TeeState, VerificationError, VerificationResponse};
use base64::Engine;
use chrono::Utc;
use common::Bytes32;
use kbs_types::Tee;
use tracing::{debug, info, warn};

/// Decode base64-encoded evidence and auto-detect TEE type.
fn decode_evidence(evidence: &str) -> Result<(Tee, serde_json::Value), VerificationError> {
    let json_bytes = base64::prelude::BASE64_STANDARD
        .decode(evidence)
        .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid base64 evidence: {e}")))?;

    let evidence_json: serde_json::Value = serde_json::from_slice(&json_bytes)
        .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid JSON evidence: {e}")))?;

    let tee = detect_tee_type(&evidence_json).ok_or_else(|| {
        VerificationError::InvalidQuoteFormat(
            "Cannot detect TEE type: evidence must contain 'quote' (TDX) or 'attestation_report' (SEV-SNP)".into(),
        )
    })?;

    debug!(tee = ?tee, "Detected TEE type from evidence format");
    Ok((tee, evidence_json))
}

/// Extract instance_id from evidence without signature verification.
/// Used for Phase 2 challenge lookup.
fn extract_instance_id(tee: Tee, evidence: &serde_json::Value) -> Result<InstanceId, VerificationError> {
    match tee {
        Tee::Tdx => {
            let quote = TdxQuote::from_json(evidence.clone())?;
            Ok(quote.instance_id())
        }
        Tee::Snp => {
            let report = SnpReport::from_json(evidence.clone())?;
            Ok(report.instance_id())
        }
        _ => Err(VerificationError::InvalidQuoteFormat(format!("Unsupported TEE type: {tee:?}"))),
    }
}

/// Phase 1: Register TEE and issue challenge.
///
/// Verifies the attestation signature, extracts the instance ID, checks the measurement
/// is in the registry whitelist, and issues a challenge nonce.
///
/// # Arguments
/// * `registry` - Peer registry for tracking attestation sessions
/// * `measurement_client` - Client for querying trusted measurements
/// * `namespace` - Namespace to register in (e.g., "guardian")
/// * `evidence` - Base64-encoded evidence string (auto-detects TEE type)
pub async fn register_tee(
    registry: &PeerRegistry,
    measurement_client: &MeasurementRegistryClient,
    namespace: &str,
    evidence: &str,
) -> Result<RegistrationResponse, VerificationError> {
    let (tee, evidence_json) = decode_evidence(evidence)?;

    debug!(namespace = %namespace, tee = ?tee, "Phase 1: Registering TEE");

    // Verify attestation based on TEE type
    let (instance_id, measurement_hash) = match tee {
        Tee::Tdx => {
            let quote = TdxQuote::from_json(evidence_json)?;

            // TDX requires non-zero instance_id (client-generated in REPORTDATA[32:64])
            if quote.instance_id().is_zero() {
                warn!(namespace = %namespace, "TDX instance has zero REPORTDATA[32:64]");
                return Err(VerificationError::IdentityRequired);
            }

            // Verify DCAP signature
            let verified = quote.verify_signature().await?;
            (verified.instance_id(), verified.measurement_hash())
        }
        Tee::Snp => {
            let report = SnpReport::from_json(evidence_json)?;

            // Verify VCEK/VLEK signature
            let verified = report.verify_signature().await?;
            (verified.instance_id(), verified.measurement_hash())
        }
        _ => {
            return Err(VerificationError::InvalidQuoteFormat(format!("Unsupported TEE type: {tee:?}")));
        }
    };

    debug!(
        instance_id = %instance_id,
        measurement_hash = %measurement_hash,
        "Phase 1: Verified quote signature"
    );

    // Query measurement registry
    let is_trusted = measurement_client
        .verify_measurement(namespace, &measurement_hash)
        .await
        .map_err(|e| VerificationError::ConnectionFailed(format!("Registry query failed: {e}")))?;

    if !is_trusted {
        warn!(
            namespace = %namespace,
            measurement_hash = %measurement_hash,
            "Measurement not in registry whitelist"
        );
        return Err(VerificationError::MeasurementNotTrusted(format!(
            "Measurement '{}...' not found in namespace '{}'",
            &measurement_hash[..measurement_hash.len().min(16)],
            namespace
        )));
    }

    // Register and get challenge
    let (challenge, is_new) = registry
        .register(instance_id, namespace.to_string(), measurement_hash, tee)
        .await
        .map_err(|_| VerificationError::IdentityConflict(instance_id.to_hex()))?;

    let expires_at = (Utc::now().timestamp() + 60) as u64;

    if is_new {
        info!(
            instance_id = %instance_id,
            namespace = %namespace,
            tee = ?tee,
            "Phase 1: Challenge issued"
        );
    }

    Ok(RegistrationResponse { challenge: hex::encode(challenge), expires_at })
}

/// Phase 2: Verify challenge-bound attestation.
///
/// Extracts instance_id from the quote, looks up the pending challenge,
/// and verifies the quote includes the correct challenge nonce.
///
/// # Arguments
/// * `registry` - Peer registry for tracking attestation sessions
/// * `namespace` - Expected namespace
/// * `evidence` - Base64-encoded evidence string
pub async fn verify_tee(
    registry: &PeerRegistry,
    namespace: &str,
    evidence: &str,
) -> Result<VerificationResponse, VerificationError> {
    let (tee, evidence_json) = decode_evidence(evidence)?;

    debug!(namespace = %namespace, tee = ?tee, "Phase 2: Verifying TEE");

    // Extract instance_id without full signature verification (for challenge lookup)
    let instance_id = extract_instance_id(tee, &evidence_json)?;

    // Look up pending challenge
    let pending = registry.get_pending_record(&instance_id).await.ok_or(VerificationError::NoPendingChallenge)?;

    if pending.namespace != namespace {
        return Err(VerificationError::NonceVerificationFailed(format!(
            "Namespace mismatch: expected '{}', got '{}'",
            pending.namespace, namespace
        )));
    }

    let expected_challenge = pending
        .get_valid_challenge()
        .ok_or_else(|| VerificationError::NonceVerificationFailed("Challenge expired".into()))?;
    let challenge_bytes = Bytes32::from(*expected_challenge);

    // Verify attestation with challenge binding based on TEE type
    let (verified_instance_id, measurement_hash, tcb_status) = match tee {
        Tee::Tdx => {
            let quote = TdxQuote::from_json(evidence_json)?;
            let verified = quote.verify_signature().await?;

            // Verify REPORTDATA[0:32] contains our challenge
            verified.verify_report_data(&challenge_bytes)?;

            (verified.instance_id(), verified.measurement_hash(), verified.tcb_status().map(String::from))
        }
        Tee::Snp => {
            let report = SnpReport::from_json(evidence_json)?;
            let verified = report.verify_signature().await?;

            // Verify REPORT_DATA[0:32] contains our challenge
            verified.verify_report_data(&challenge_bytes)?;

            (verified.instance_id(), verified.measurement_hash(), verified.tcb_status().map(String::from))
        }
        _ => {
            return Err(VerificationError::InvalidQuoteFormat(format!("Unsupported TEE type: {tee:?}")));
        }
    };

    // Verify instance_id matches (defense in depth)
    if verified_instance_id != instance_id {
        return Err(VerificationError::NonceVerificationFailed("Instance ID mismatch between phases".into()));
    }

    // Complete verification
    let state = TeeState::Verified {
        measurement_hash: measurement_hash.clone(),
        tcb_status: tcb_status.clone().unwrap_or_else(|| "Unknown".to_string()),
        tee,
        verified_at: Utc::now(),
    };

    registry
        .complete_verification(instance_id, state)
        .await
        .ok_or_else(|| VerificationError::AttestationFailed("Failed to complete verification".into()))?;

    info!(
        instance_id = %instance_id,
        namespace = %namespace,
        tee = ?tee,
        "Phase 2: TEE verified"
    );

    Ok(VerificationResponse { verified: true, measurement_hash: Some(measurement_hash), tcb_status })
}

// Note: detect_tee_type tests are in backend/mod.rs where the function is defined
