// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! TEE attestation verification backends.
//!
//! This module provides platform-specific attestation parsing and verification:
//! - [`TdxQuote`] / [`VerifiedTdxQuote`] - Intel TDX (DCAP verification)
//! - [`SnpReport`] / [`VerifiedSnpReport`] - AMD SEV-SNP (VCEK/VLEK verification)
//!
//! # Architecture
//!
//! ```text
//! Evidence (base64) ──► TdxQuote::from_base64() ──► TdxQuote (parsed)
//!                                                        │
//!                                                        ▼ verify_signature()
//!                                                   VerifiedTdxQuote
//!                                                        │
//!                                      ┌─────────────────┼─────────────────┐
//!                                      ▼                 ▼                 ▼
//!                              verify_report_data()  verify_tcb()  verify_measurements()
//! ```
//!
//! # Usage Examples
//!
//! ## Lightweight parsing (no signature verification)
//!
//! ```ignore
//! // Parse to extract instance_id without crypto overhead
//! let quote = TdxQuote::from_base64(evidence)?;
//! let instance_id = quote.instance_id();
//! ```
//!
//! ## Gossip verification (signature + binding only)
//!
//! ```ignore
//! let quote = TdxQuote::from_base64(evidence)?;
//! let verified = quote.verify_signature().await?;
//! verified.verify_report_data(&expected_binding)?;
//! ```
//!
//! ## Full attestation verification
//!
//! ```ignore
//! let quote = TdxQuote::from_base64(evidence)?;
//! let verified = quote.verify_signature().await?;
//! verified.verify_report_data(&challenge)?;
//! verified.verify_debug(trusted.debug_allowed)?;
//! if let Some(min_tcb) = &trusted.minimum_tcb {
//!     verified.verify_tcb(min_tcb)?;
//! }
//! verified.verify_measurements(trusted)?;
//! ```

mod sev_snp;
mod tdx;

pub use sev_snp::{SnpReport, VerifiedSnpReport};
pub use tdx::{TdAttributes, TdxQuote, VerifiedTdxQuote};

use crate::types::VerificationError;
use base64::Engine;
use kbs_types::Tee;
use std::path::Path;

/// Check if a TEE platform is available on this system.
///
/// # Platform-specific checks
/// - **TDX**: Checks for `/sys/kernel/config/tsm/report` or `/dev/tdx_guest`
/// - **SEV-SNP**: Checks for `/dev/sev-guest`
#[must_use]
pub fn is_tee_available(tee: Tee) -> bool {
    match tee {
        Tee::Tdx => Path::new("/sys/kernel/config/tsm/report").exists() || Path::new("/dev/tdx_guest").exists(),
        Tee::Snp => Path::new("/dev/sev-guest").exists(),
        _ => false,
    }
}

/// Detect which TEE platform is available on this system.
///
/// Returns the first available TEE platform, preferring TDX over SEV-SNP.
/// Returns `None` if no TEE platform is detected.
#[must_use]
pub fn detect_available_tee() -> Option<Tee> {
    if is_tee_available(Tee::Tdx) {
        Some(Tee::Tdx)
    } else if is_tee_available(Tee::Snp) {
        Some(Tee::Snp)
    } else {
        None
    }
}

/// Detect TEE type from evidence JSON structure.
///
/// - TDX evidence contains a `"quote"` field
/// - SEV-SNP evidence contains an `"attestation_report"` field
#[must_use]
pub fn detect_tee_type(evidence: &serde_json::Value) -> Option<Tee> {
    if evidence.get("quote").is_some() {
        Some(Tee::Tdx)
    } else if evidence.get("attestation_report").is_some() {
        Some(Tee::Snp)
    } else {
        None
    }
}

/// Generate TEE attestation evidence with the given REPORTDATA.
///
/// Returns the evidence as a JSON Value (not base64-encoded).
///
/// # Errors
///
/// Returns an error if:
/// - The TEE type is not supported
/// - Attestation generation fails
pub async fn generate_evidence(tee: Tee, report_data: &[u8; 64]) -> Result<serde_json::Value, VerificationError> {
    match tee {
        Tee::Tdx => {
            use attester::{tdx::TdxAttester, Attester};
            let attester = TdxAttester::default();
            attester
                .get_evidence(report_data.to_vec())
                .await
                .map_err(|e| VerificationError::AttestationFailed(format!("TDX attestation failed: {e}")))
        }
        Tee::Snp => {
            use attester::{snp::SnpAttester, Attester};
            let attester = SnpAttester::default();
            attester
                .get_evidence(report_data.to_vec())
                .await
                .map_err(|e| VerificationError::AttestationFailed(format!("SNP attestation failed: {e}")))
        }
        _ => Err(VerificationError::InvalidQuoteFormat(format!("Unsupported TEE type: {tee:?}"))),
    }
}

/// Extract measurement hash from base64-encoded evidence JSON.
///
/// Works for both TDX quotes and SNP reports. Auto-detects the TEE type
/// from the evidence structure.
///
/// # Errors
///
/// Returns an error if:
/// - The base64 decoding fails
/// - The JSON parsing fails
/// - The TEE type cannot be detected
/// - The quote/report parsing fails
pub fn measurement_hash_from_evidence(evidence_b64: &str) -> Result<String, VerificationError> {
    let evidence_bytes = base64::engine::general_purpose::STANDARD
        .decode(evidence_b64)
        .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid evidence base64: {e}")))?;

    let evidence_json: serde_json::Value = serde_json::from_slice(&evidence_bytes)
        .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid evidence JSON: {e}")))?;

    measurement_hash_from_json(&evidence_json)
}

/// Extract measurement hash from evidence JSON Value.
///
/// Works for both TDX quotes and SNP reports.
///
/// # Errors
///
/// Returns an error if the TEE type cannot be detected or parsing fails.
pub fn measurement_hash_from_json(evidence: &serde_json::Value) -> Result<String, VerificationError> {
    let tee = detect_tee_type(evidence)
        .ok_or_else(|| VerificationError::InvalidQuoteFormat("Cannot detect TEE type".into()))?;

    match tee {
        Tee::Tdx => {
            let quote = TdxQuote::from_json(evidence.clone())?;
            Ok(quote.measurement_hash())
        }
        Tee::Snp => {
            let report = SnpReport::from_json(evidence.clone())?;
            Ok(report.measurement_hash())
        }
        _ => Err(VerificationError::InvalidQuoteFormat(format!("Unsupported TEE: {tee:?}"))),
    }
}

/// Generate fresh attestation and return its measurement hash.
///
/// Uses zero-filled REPORTDATA since only measurements are needed.
/// This is useful for startup verification of the node's own measurements.
///
/// # Errors
///
/// Returns an error if attestation generation or parsing fails.
pub async fn get_measurement_hash(tee: Tee) -> Result<String, VerificationError> {
    let evidence = generate_evidence(tee, &[0u8; 64]).await?;
    measurement_hash_from_json(&evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_tee_type_tdx() {
        let evidence = serde_json::json!({ "quote": "base64data" });
        assert_eq!(detect_tee_type(&evidence), Some(Tee::Tdx));
    }

    #[test]
    fn test_detect_tee_type_snp() {
        let evidence = serde_json::json!({ "attestation_report": {} });
        assert_eq!(detect_tee_type(&evidence), Some(Tee::Snp));
    }

    #[test]
    fn test_detect_tee_type_unknown() {
        let evidence = serde_json::json!({ "unknown": "data" });
        assert_eq!(detect_tee_type(&evidence), None);
    }
}
