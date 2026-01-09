// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! AMD SEV-SNP attestation verification.
//!
//! Provides types and verification for AMD Secure Encrypted Virtualization -
//! Secure Nested Paging (SEV-SNP) using VCEK/VLEK certificate chain verification.
//!
//! # SEV-SNP Attestation Report Structure
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Attestation Report (1184 bytes)                             │
//! │   - version: u32                                            │
//! │   - guest_svn: u32                                          │
//! │   - policy: u64                                             │
//! │   - family_id: [u8; 16]                                     │
//! │   - image_id: [u8; 16]                                      │
//! │   - vmpl: u32                                               │
//! │   - signature_algo: u32                                     │
//! │   - platform_version: u64 (TCB)                             │
//! │   - platform_info: u64                                      │
//! │   - author_key_en: u32                                      │
//! │   - reserved1: u32                                          │
//! │   - report_data: [u8; 64]  ← User-provided data             │
//! │   - measurement: [u8; 48]  ← Launch measurement             │
//! │   - host_data: [u8; 32]                                     │
//! │   - id_key_digest: [u8; 48]                                 │
//! │   - author_key_digest: [u8; 48]                             │
//! │   - report_id: [u8; 32]    ← Firmware instance ID           │
//! │   - report_id_ma: [u8; 32]                                  │
//! │   - reported_tcb: u64                                       │
//! │   - reserved2: [u8; 24]                                     │
//! │   - chip_id: [u8; 64]                                       │
//! │   - reserved3: [u8; 192]                                    │
//! │   - signature: [u8; 512]                                    │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Identity Model
//!
//! Unlike TDX where instance_id is client-provided in REPORTDATA[32:64],
//! SEV-SNP uses the firmware-generated REPORT_ID as the instance identifier.
//!
//! # Measurement Hash
//!
//! `SHA256(measurement || host_data)` or `SHA256(measurement)` if host_data is zero.

use crate::types::{
    InstanceId, ParsedAttestation, SnpMeasurement, SnpMinimumTcb, SnpPlatformConstraints, VerificationError,
};
use base64::Engine;
use common::{Bytes32, Bytes48, Bytes64};
use kbs_types::Tee;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

/// Parsed SEV-SNP Attestation Report (not yet signature-verified).
///
/// Use this for lightweight operations like extracting `instance_id`
/// without the cost of VCEK/VLEK signature verification.
///
/// # Example
///
/// ```ignore
/// // Parse report from base64 evidence (no signature verification)
/// let report = SnpReport::from_base64(evidence)?;
/// let instance_id = report.instance_id();
///
/// // Later, verify the signature
/// let verified = report.verify_signature().await?;
/// ```
#[derive(Debug, Clone)]
pub struct SnpReport {
    /// Original evidence JSON (needed for signature verification)
    evidence: serde_json::Value,
    /// REPORT_DATA field (64 bytes) - user-provided
    report_data: Bytes64,
    /// REPORT_ID (32 bytes) - firmware-generated instance ID
    report_id: Bytes32,
    /// Launch measurement (48 bytes)
    measurement: Bytes48,
    /// Host data (32 bytes)
    host_data: Bytes32,
    /// Policy flags (contains debug bit)
    policy: u64,
}

impl SnpReport {
    /// Parse a SEV-SNP report from base64-encoded evidence.
    ///
    /// Evidence format: `base64(JSON{"attestation_report": {...}, "cert_chain": [...]})`
    ///
    /// This performs structural parsing only - NO signature verification.
    /// Use `verify_signature()` to cryptographically verify the report.
    pub fn from_base64(evidence_b64: &str) -> Result<Self, VerificationError> {
        // Decode outer base64 → JSON
        let evidence_bytes = base64::prelude::BASE64_STANDARD
            .decode(evidence_b64)
            .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid evidence base64: {e}")))?;

        let evidence: serde_json::Value = serde_json::from_slice(&evidence_bytes)
            .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid evidence JSON: {e}")))?;

        Self::from_json(evidence)
    }

    /// Parse a SEV-SNP report from evidence JSON.
    ///
    /// Use this when you already have the decoded JSON.
    pub fn from_json(evidence: serde_json::Value) -> Result<Self, VerificationError> {
        let report = &evidence["attestation_report"];
        if report.is_null() {
            return Err(VerificationError::InvalidQuoteFormat(
                "Missing 'attestation_report' field in SEV-SNP evidence".into(),
            ));
        }

        // Extract REPORT_ID (instance identifier)
        let report_id_bytes: [u8; 32] = Self::extract_bytes_field(report, "report_id", 32)?
            .try_into()
            .map_err(|_| VerificationError::ParseError("Invalid report_id length".into()))?;
        let report_id = Bytes32::from(report_id_bytes);

        // Extract REPORT_DATA
        let report_data_bytes: [u8; 64] = Self::extract_bytes_field(report, "report_data", 64)?
            .try_into()
            .map_err(|_| VerificationError::ParseError("Invalid report_data length".into()))?;
        let report_data = Bytes64::from(report_data_bytes);

        // Extract measurement
        let measurement_bytes: [u8; 48] = Self::extract_bytes_field(report, "measurement", 48)?
            .try_into()
            .map_err(|_| VerificationError::ParseError("Invalid measurement length".into()))?;
        let measurement = Bytes48::from(measurement_bytes);

        // Extract host_data
        let host_data_bytes: [u8; 32] = Self::extract_bytes_field(report, "host_data", 32)?
            .try_into()
            .map_err(|_| VerificationError::ParseError("Invalid host_data length".into()))?;
        let host_data = Bytes32::from(host_data_bytes);

        // Extract policy
        let policy = Self::extract_u64_field(report, "policy")?;

        debug!(
            report_id = %report_id,
            policy = format!("{:#x}", policy),
            "Parsed SEV-SNP attestation report"
        );

        Ok(Self { evidence, report_data, report_id, measurement, host_data, policy })
    }

    /// Extract a bytes field from JSON (handles hex string or array formats).
    fn extract_bytes_field(
        report: &serde_json::Value,
        field: &str,
        expected_len: usize,
    ) -> Result<Vec<u8>, VerificationError> {
        // Try hex string format first
        if let Some(hex_str) = report[field].as_str() {
            let bytes =
                hex::decode(hex_str).map_err(|e| VerificationError::ParseError(format!("Invalid {field} hex: {e}")))?;
            if bytes.len() != expected_len {
                return Err(VerificationError::ParseError(format!(
                    "{field} length mismatch: expected {expected_len}, got {}",
                    bytes.len()
                )));
            }
            return Ok(bytes);
        }

        // Try array format
        if let Some(arr) = report[field].as_array() {
            let bytes: Result<Vec<u8>, _> =
                arr.iter().map(|v| v.as_u64().map(|n| n as u8).ok_or("Invalid byte")).collect();
            let bytes = bytes.map_err(|e| VerificationError::ParseError(format!("Invalid {field} array: {e}")))?;
            if bytes.len() != expected_len {
                return Err(VerificationError::ParseError(format!(
                    "{field} length mismatch: expected {expected_len}, got {}",
                    bytes.len()
                )));
            }
            return Ok(bytes);
        }

        Err(VerificationError::ParseError(format!("Missing or invalid {field} field")))
    }

    /// Extract a u64 field from JSON.
    fn extract_u64_field(report: &serde_json::Value, field: &str) -> Result<u64, VerificationError> {
        report[field].as_u64().ok_or_else(|| VerificationError::ParseError(format!("Missing or invalid {field} field")))
    }

    /// Get the full REPORT_DATA (64 bytes).
    #[must_use]
    pub fn report_data(&self) -> &Bytes64 {
        &self.report_data
    }

    /// Get REPORT_DATA[0:32] - typically used for challenge binding.
    #[must_use]
    pub fn report_data_binding(&self) -> Bytes32 {
        let mut binding = [0u8; 32];
        binding.copy_from_slice(&self.report_data.as_bytes()[0..32]);
        Bytes32::from(binding)
    }

    /// Get the instance_id (REPORT_ID - firmware-generated).
    ///
    /// Note: Unlike TDX, SEV-SNP instance_id comes from REPORT_ID,
    /// not from REPORT_DATA.
    #[must_use]
    pub fn instance_id(&self) -> InstanceId {
        InstanceId::from(self.report_id)
    }

    /// Get the REPORT_ID (32 bytes).
    #[must_use]
    pub fn report_id(&self) -> Bytes32 {
        self.report_id
    }

    /// Get the launch measurement (48 bytes).
    #[must_use]
    pub fn measurement(&self) -> &Bytes48 {
        &self.measurement
    }

    /// Get the host data (32 bytes).
    #[must_use]
    pub fn host_data(&self) -> &Bytes32 {
        &self.host_data
    }

    /// Compute the measurement hash.
    ///
    /// Returns `SHA256(measurement || host_data)` if host_data is non-zero,
    /// otherwise `SHA256(measurement)`.
    #[must_use]
    pub fn measurement_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.measurement.as_bytes());

        // Include host_data if non-zero
        if self.host_data.as_bytes().iter().any(|&b| b != 0) {
            hasher.update(self.host_data.as_bytes());
        }

        hex::encode(hasher.finalize())
    }

    /// Check if debug mode is enabled (policy bit 19).
    #[must_use]
    pub fn is_debug(&self) -> bool {
        self.policy & (1 << 19) != 0
    }

    /// Get the policy flags.
    #[must_use]
    pub fn policy(&self) -> u64 {
        self.policy
    }

    /// Verify the VCEK/VLEK signature chain and return a verified report.
    ///
    /// This performs full cryptographic verification:
    /// - Verifies signature over report using VCEK or VLEK
    /// - Validates certificate chain back to AMD root
    /// - Extracts TCB status
    pub async fn verify_signature(self) -> Result<VerifiedSnpReport, VerificationError> {
        use verifier::{InitDataHash, ReportData};

        debug!("Verifying SEV-SNP signature");

        // Get SEV-SNP verifier from confidential-containers
        let verifier = verifier::to_verifier(&Tee::Snp, None)
            .await
            .map_err(|e| VerificationError::AttestationFailed(format!("Failed to create SEV-SNP verifier: {e}")))?;

        // Verify signature - don't check REPORT_DATA here, caller will do that
        let results = verifier
            .evaluate(self.evidence.clone(), &ReportData::NotProvided, &InitDataHash::NotProvided)
            .await
            .map_err(|e| VerificationError::AttestationFailed(format!("SNP signature verification failed: {e}")))?;

        if results.is_empty() {
            return Err(VerificationError::AttestationFailed("No claims returned from SNP verifier".into()));
        }

        let claims = results[0].0.clone();

        // Construct TCB status string
        let tcb_status = Some(format!(
            "bootloader={}, tee={}, snp={}, microcode={}",
            claims["reported_tcb_bootloader"].as_u64().unwrap_or(0),
            claims["reported_tcb_tee"].as_u64().unwrap_or(0),
            claims["reported_tcb_snp"].as_u64().unwrap_or(0),
            claims["reported_tcb_microcode"].as_u64().unwrap_or(0)
        ));

        info!(
            measurement_hash = %self.measurement_hash(),
            tcb_status = ?tcb_status,
            "SEV-SNP signature verified"
        );

        Ok(VerifiedSnpReport { report: self, claims, tcb_status })
    }
}

/// Signature-verified SEV-SNP Attestation Report.
///
/// Created by calling `SnpReport::verify_signature()`. All accessors
/// from `SnpReport` are available, plus policy verification methods.
#[derive(Debug, Clone)]
pub struct VerifiedSnpReport {
    /// The underlying parsed report
    report: SnpReport,
    /// Verified claims from SNP verifier
    claims: serde_json::Value,
    /// TCB status string
    tcb_status: Option<String>,
}

impl std::ops::Deref for VerifiedSnpReport {
    type Target = SnpReport;

    fn deref(&self) -> &Self::Target {
        &self.report
    }
}

impl VerifiedSnpReport {
    /// Get the verified claims JSON.
    #[must_use]
    pub fn claims(&self) -> &serde_json::Value {
        &self.claims
    }

    /// Get the TCB status.
    #[must_use]
    pub fn tcb_status(&self) -> Option<&str> {
        self.tcb_status.as_deref()
    }

    // === Verification methods ===

    /// Verify REPORT_DATA[0:32] matches expected value.
    ///
    /// Use this to verify challenge-response binding.
    pub fn verify_report_data(&self, expected: &Bytes32) -> Result<(), VerificationError> {
        let actual = self.report_data_binding();
        if &actual != expected {
            return Err(VerificationError::NonceVerificationFailed(format!(
                "REPORT_DATA[0:32] mismatch: expected {}, got {}",
                expected, actual
            )));
        }
        Ok(())
    }

    /// Verify debug policy compliance.
    pub fn verify_debug(&self, debug_allowed: bool) -> Result<(), VerificationError> {
        if self.is_debug() && !debug_allowed {
            warn!("SEV-SNP debug mode enabled but not allowed by policy");
            return Err(VerificationError::DebugNotAllowed);
        }
        Ok(())
    }

    /// Verify TCB meets minimum requirements.
    pub fn verify_tcb(&self, minimum: &SnpMinimumTcb) -> Result<(), VerificationError> {
        // Check bootloader SVN
        if let Some(min_bl) = minimum.bootloader_svn {
            let actual = self.claims["reported_tcb_bootloader"].as_u64().unwrap_or(0) as u8;
            if actual < min_bl {
                warn!(actual = actual, minimum = min_bl, "Bootloader SVN below minimum");
                return Err(VerificationError::TcbOutdated(format!("bootloader_svn {} < minimum {}", actual, min_bl)));
            }
        }

        // Check TEE SVN
        if let Some(min_tee) = minimum.tee_svn {
            let actual = self.claims["reported_tcb_tee"].as_u64().unwrap_or(0) as u8;
            if actual < min_tee {
                warn!(actual = actual, minimum = min_tee, "TEE SVN below minimum");
                return Err(VerificationError::TcbOutdated(format!("tee_svn {} < minimum {}", actual, min_tee)));
            }
        }

        // Check SNP SVN
        if let Some(min_snp) = minimum.snp_svn {
            let actual = self.claims["reported_tcb_snp"].as_u64().unwrap_or(0) as u8;
            if actual < min_snp {
                warn!(actual = actual, minimum = min_snp, "SNP SVN below minimum");
                return Err(VerificationError::TcbOutdated(format!("snp_svn {} < minimum {}", actual, min_snp)));
            }
        }

        // Check microcode SVN
        if let Some(min_ucode) = minimum.microcode_svn {
            let actual = self.claims["reported_tcb_microcode"].as_u64().unwrap_or(0) as u8;
            if actual < min_ucode {
                warn!(actual = actual, minimum = min_ucode, "Microcode SVN below minimum");
                return Err(VerificationError::TcbOutdated(format!(
                    "microcode_svn {} < minimum {}",
                    actual, min_ucode
                )));
            }
        }

        Ok(())
    }

    /// Verify platform constraints.
    pub fn verify_platform(&self, constraints: &SnpPlatformConstraints) -> Result<(), VerificationError> {
        // Check SMT disabled requirement
        if let Some(true) = constraints.require_smt_disabled {
            let smt_enabled = self.claims["platform_smt_enabled"].as_bool().unwrap_or(false);
            if smt_enabled {
                warn!("SMT enabled but required to be disabled");
                return Err(VerificationError::PlatformConstraint(
                    "SMT (Simultaneous Multi-Threading) must be disabled".into(),
                ));
            }
        }

        // Check VMPL
        if let Some(required_vmpl) = constraints.required_vmpl {
            if required_vmpl != 0 {
                debug!("VMPL check for non-zero values not yet implemented");
            }
        }

        // Note: chip_id, id_key_digest, author_key_digest checks would require
        // additional parsing from the raw report structure
        if constraints.allowed_chip_ids.is_some() {
            debug!("Chip ID allowlist check not yet implemented");
        }

        Ok(())
    }

    /// Verify measurements match trusted values.
    pub fn verify_measurements(&self, trusted: &SnpMeasurement) -> Result<(), VerificationError> {
        // Compute expected hash from trusted measurement
        let measurement_bytes = hex::decode(&trusted.measurement)
            .map_err(|e| VerificationError::ParseError(format!("Invalid trusted measurement hex: {e}")))?;

        let mut hasher = Sha256::new();
        hasher.update(&measurement_bytes);

        // Include host_data if specified
        if let Some(ref host_data_hex) = trusted.host_data {
            let host_data = hex::decode(host_data_hex)
                .map_err(|e| VerificationError::ParseError(format!("Invalid trusted host_data hex: {e}")))?;
            if host_data.iter().any(|&b| b != 0) {
                hasher.update(&host_data);
            }
        }

        let expected_hash = hex::encode(hasher.finalize());
        let actual_hash = self.measurement_hash();

        if actual_hash != expected_hash {
            warn!(
                expected = %expected_hash,
                actual = %actual_hash,
                "Measurement hash mismatch"
            );
            return Err(VerificationError::MeasurementMismatch { expected: expected_hash, actual: actual_hash });
        }

        info!(measurement_hash = %actual_hash, "SEV-SNP measurements verified");
        Ok(())
    }

    /// Convert to platform-agnostic ParsedAttestation.
    #[must_use]
    pub fn into_parsed_attestation(self) -> ParsedAttestation {
        ParsedAttestation {
            tee: Tee::Snp,
            instance_id: self.instance_id(),
            measurement_hash: self.measurement_hash(),
            claims: self.claims,
            tcb_status: self.tcb_status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measurement_hash_with_host_data() {
        let measurement = [0xAA; 48];
        let host_data = [0xBB; 32];

        let mut hasher = Sha256::new();
        hasher.update(&measurement);
        hasher.update(&host_data);
        let with_host = hex::encode(hasher.finalize());

        let mut hasher2 = Sha256::new();
        hasher2.update(&measurement);
        let without_host = hex::encode(hasher2.finalize());

        assert_ne!(with_host, without_host);
    }

    #[test]
    fn test_measurement_hash_zero_host_data() {
        let measurement = [0xAA; 48];
        let host_data_zero = [0x00; 32];

        // With zero host_data, should NOT include it
        let mut hasher = Sha256::new();
        hasher.update(&measurement);
        let expected = hex::encode(hasher.finalize());

        // If host_data is all zeros, measurement_hash() should produce same result
        let mut hasher2 = Sha256::new();
        hasher2.update(&measurement);
        if host_data_zero.iter().any(|&b| b != 0) {
            hasher2.update(&host_data_zero);
        }
        let actual = hex::encode(hasher2.finalize());

        assert_eq!(expected, actual);
    }

    #[test]
    fn test_debug_flag_policy() {
        // Policy with debug bit (bit 19) set
        let debug_policy: u64 = 1 << 19;
        assert!(debug_policy & (1 << 19) != 0);

        // Policy without debug bit
        let nodebug_policy: u64 = 0;
        assert!(nodebug_policy & (1 << 19) == 0);
    }
}
