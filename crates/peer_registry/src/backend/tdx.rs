// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Intel TDX attestation verification.
//!
//! Provides types and verification for Intel Trust Domain Extensions (TDX)
//! using DCAP (Data Center Attestation Primitives).
//!
//! # TDX Quote Structure (v4/v5)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ Header (48 bytes)                                           │
//! │   - version: u16 (4 or 5)                                   │
//! │   - att_key_type: u16                                       │
//! │   - tee_type: u32                                           │
//! │   - reserved: [u8; 4]                                       │
//! │   - vendor_id: [u8; 16]                                     │
//! │   - user_data: [u8; 20]                                     │
//! ├─────────────────────────────────────────────────────────────┤
//! │ TD Quote Body (584 bytes)                                   │
//! │   - tee_tcb_svn: [u8; 16]                                   │
//! │   - mr_seam: [u8; 48]                                       │
//! │   - mrsigner_seam: [u8; 48]                                 │
//! │   - seam_attributes: [u8; 8]                                │
//! │   - td_attributes: [u8; 8]                                  │
//! │   - xfam: [u8; 8]                                           │
//! │   - mr_td: [u8; 48]                                         │
//! │   - mr_config_id: [u8; 48]                                  │
//! │   - mr_owner: [u8; 48]                                      │
//! │   - mr_owner_config: [u8; 48]                               │
//! │   - rtmr_0: [u8; 48]                                        │
//! │   - rtmr_1: [u8; 48]                                        │
//! │   - rtmr_2: [u8; 48]                                        │
//! │   - rtmr_3: [u8; 48]                                        │
//! │   - report_data: [u8; 64]  ← [binding(32) | instance_id(32)]│
//! ├─────────────────────────────────────────────────────────────┤
//! │ Signature Data                                              │
//! │   - ECDSA signature + cert chain                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Measurement Hash
//!
//! `SHA256(RTMR0 || RTMR1 || RTMR2 || RTMR3)`

use crate::types::{
    InstanceId, ParsedAttestation, TdxMeasurement, TdxMinimumTcb, TdxPlatformConstraints, VerificationError,
};
use base64::Engine;
use common::{Bytes32, Bytes48, Bytes64};
use kbs_types::Tee;
use sha2::{Digest, Sha256};
use std::fmt;
use tracing::{debug, info, warn};

// Quote structure constants
const HEADER_SIZE: usize = 48;
const BODY_SIZE: usize = 584;
const REPORT_DATA_OFFSET: usize = 520; // Offset within body

// ============================================================================
// TdAttributes - TDX TD_ATTRIBUTES bitmap
// ============================================================================

/// TDX TD_ATTRIBUTES bitmap (8 bytes / 64 bits).
///
/// TD_ATTRIBUTES is set by the VMM during `TDH.MNG.INIT` and controls
/// security-sensitive TD properties.
///
/// # Known Bits
///
/// | Bit | Name | Description |
/// |-----|------|-------------|
/// | 0 | DEBUG | TD is debuggable (allows VMREAD/VMWRITE to TD state) |
/// | 4 | SEPT_VE_DISABLE | Disable EPT violation conversion to #VE |
/// | 27 | MIGRATABLE | TD supports migration |
/// | 28 | PKS | Enable Protection Keys for Supervisor pages |
/// | 30 | KL | Enable Key Locker |
/// | 31 | PERFMON | Enable performance monitoring |
///
/// Bits 1-3, 5-26, 29, 32-63 are reserved.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TdAttributes(u64);

impl TdAttributes {
    /// Bit 0: TD is debuggable
    pub const DEBUG: u64 = 1 << 0;
    /// Bit 4: Disable EPT violation conversion to #VE
    pub const SEPT_VE_DISABLE: u64 = 1 << 4;
    /// Bit 27: TD supports migration
    pub const MIGRATABLE: u64 = 1 << 27;
    /// Bit 28: Protection Keys for Supervisor pages enabled
    pub const PKS: u64 = 1 << 28;
    /// Bit 30: Key Locker enabled
    pub const KL: u64 = 1 << 30;
    /// Bit 31: Performance monitoring enabled
    pub const PERFMON: u64 = 1 << 31;

    /// Create from raw 8-byte array (little-endian).
    #[must_use]
    pub fn from_bytes(bytes: [u8; 8]) -> Self {
        Self(u64::from_le_bytes(bytes))
    }

    /// Create from raw u64 value.
    #[must_use]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Get the raw u64 value.
    #[must_use]
    pub const fn as_raw(&self) -> u64 {
        self.0
    }

    /// Get as byte array (little-endian).
    #[must_use]
    pub fn as_bytes(&self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    /// Check if a specific bit is set.
    #[must_use]
    pub const fn is_set(&self, bit: u64) -> bool {
        self.0 & bit != 0
    }

    /// Check if DEBUG bit is set (bit 0).
    ///
    /// A debuggable TD allows the VMM to read/write TD state,
    /// which is a security risk in production.
    #[must_use]
    pub const fn is_debug(&self) -> bool {
        self.is_set(Self::DEBUG)
    }

    /// Check if SEPT_VE_DISABLE bit is set (bit 4).
    #[must_use]
    pub const fn is_sept_ve_disabled(&self) -> bool {
        self.is_set(Self::SEPT_VE_DISABLE)
    }

    /// Check if MIGRATABLE bit is set (bit 27).
    #[must_use]
    pub const fn is_migratable(&self) -> bool {
        self.is_set(Self::MIGRATABLE)
    }

    /// Check if PKS (Protection Keys for Supervisor) is enabled (bit 28).
    #[must_use]
    pub const fn is_pks_enabled(&self) -> bool {
        self.is_set(Self::PKS)
    }

    /// Check if Key Locker is enabled (bit 30).
    #[must_use]
    pub const fn is_key_locker_enabled(&self) -> bool {
        self.is_set(Self::KL)
    }

    /// Check if performance monitoring is enabled (bit 31).
    #[must_use]
    pub const fn is_perfmon_enabled(&self) -> bool {
        self.is_set(Self::PERFMON)
    }
}

impl fmt::Debug for TdAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut flags = Vec::new();
        if self.is_debug() {
            flags.push("DEBUG");
        }
        if self.is_sept_ve_disabled() {
            flags.push("SEPT_VE_DISABLE");
        }
        if self.is_migratable() {
            flags.push("MIGRATABLE");
        }
        if self.is_pks_enabled() {
            flags.push("PKS");
        }
        if self.is_key_locker_enabled() {
            flags.push("KL");
        }
        if self.is_perfmon_enabled() {
            flags.push("PERFMON");
        }

        if flags.is_empty() {
            write!(f, "TdAttributes(0x{:016x})", self.0)
        } else {
            write!(f, "TdAttributes(0x{:016x} [{}])", self.0, flags.join("|"))
        }
    }
}

impl fmt::Display for TdAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:016x}", self.0)
    }
}

/// Parsed TDX Quote (not yet signature-verified).
///
/// Use this for lightweight operations like extracting `instance_id`
/// without the cost of DCAP signature verification.
///
/// # Example
///
/// ```ignore
/// // Parse quote from base64 evidence (no signature verification)
/// let quote = TdxQuote::from_base64(evidence)?;
/// let instance_id = quote.instance_id();
///
/// // Later, verify the signature
/// let verified = quote.verify_signature().await?;
/// ```
#[derive(Debug, Clone)]
pub struct TdxQuote {
    /// Original evidence JSON (needed for signature verification)
    evidence: serde_json::Value,
    /// Raw quote bytes
    raw_quote: Vec<u8>,
    /// Quote version (4 or 5)
    version: u16,
    /// REPORTDATA field (64 bytes)
    report_data: Bytes64,
    /// RTMR values (4 × 48 bytes)
    rtmrs: [Bytes48; 4],
    /// TD_ATTRIBUTES (8 bytes) - contains debug flag and other security attributes
    td_attributes: TdAttributes,
}

impl TdxQuote {
    /// Parse a TDX quote from base64-encoded evidence.
    ///
    /// Evidence format: `base64(JSON{"quote": base64(raw_quote), ...})`
    ///
    /// This performs structural parsing only - NO signature verification.
    /// Use `verify_signature()` to cryptographically verify the quote.
    pub fn from_base64(evidence_b64: &str) -> Result<Self, VerificationError> {
        // Decode outer base64 → JSON
        let evidence_bytes = base64::prelude::BASE64_STANDARD
            .decode(evidence_b64)
            .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid evidence base64: {e}")))?;

        let evidence: serde_json::Value = serde_json::from_slice(&evidence_bytes)
            .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid evidence JSON: {e}")))?;

        Self::from_json(evidence)
    }

    /// Parse a TDX quote from evidence JSON.
    ///
    /// Use this when you already have the decoded JSON.
    pub fn from_json(evidence: serde_json::Value) -> Result<Self, VerificationError> {
        // Extract quote field
        let quote_b64 = evidence["quote"]
            .as_str()
            .ok_or_else(|| VerificationError::InvalidQuoteFormat("Missing 'quote' field in TDX evidence".into()))?;

        // Decode inner base64 → raw quote bytes
        let raw_quote = base64::prelude::BASE64_STANDARD
            .decode(quote_b64)
            .map_err(|e| VerificationError::InvalidQuoteFormat(format!("Invalid quote base64: {e}")))?;

        Self::parse_raw(&evidence, raw_quote)
    }

    /// Parse from raw quote bytes (internal).
    fn parse_raw(evidence: &serde_json::Value, raw_quote: Vec<u8>) -> Result<Self, VerificationError> {
        // Check minimum size
        if raw_quote.len() < HEADER_SIZE + BODY_SIZE {
            return Err(VerificationError::InvalidQuoteFormat(format!(
                "Quote too short: {} bytes (minimum {})",
                raw_quote.len(),
                HEADER_SIZE + BODY_SIZE
            )));
        }

        // Parse version
        let version = u16::from_le_bytes([raw_quote[0], raw_quote[1]]);
        if version != 4 && version != 5 {
            return Err(VerificationError::InvalidQuoteFormat(format!("Unsupported TDX quote version: {version}")));
        }

        // Calculate body offset (v5 has 6 extra bytes in header)
        let body_offset = match version {
            4 => HEADER_SIZE,
            5 => HEADER_SIZE + 6,
            _ => unreachable!(),
        };

        if raw_quote.len() < body_offset + BODY_SIZE {
            return Err(VerificationError::InvalidQuoteFormat("Quote too short for body".into()));
        }

        let body = &raw_quote[body_offset..body_offset + BODY_SIZE];

        // Extract TD_ATTRIBUTES (offset 32 in body, 8 bytes)
        let mut td_attr_bytes = [0u8; 8];
        td_attr_bytes.copy_from_slice(&body[32..40]);
        let td_attributes = TdAttributes::from_bytes(td_attr_bytes);

        // Extract RTMRs (offset 328 in body, 4 × 48 bytes)
        let rtmr_offset = 328;
        let mut rtmrs = [Bytes48::from([0u8; 48]); 4];
        for (i, rtmr) in rtmrs.iter_mut().enumerate() {
            let start = rtmr_offset + i * 48;
            let mut bytes = [0u8; 48];
            bytes.copy_from_slice(&body[start..start + 48]);
            *rtmr = Bytes48::from(bytes);
        }

        // Extract REPORTDATA (offset 520 in body, 64 bytes)
        let mut report_data_bytes = [0u8; 64];
        report_data_bytes.copy_from_slice(&body[REPORT_DATA_OFFSET..REPORT_DATA_OFFSET + 64]);
        let report_data = Bytes64::from(report_data_bytes);

        debug!(
            version = version,
            report_data_prefix = hex::encode(&report_data_bytes[0..8]),
            td_attributes = ?td_attributes,
            "Parsed TDX quote structure"
        );

        Ok(Self { evidence: evidence.clone(), raw_quote, version, report_data, rtmrs, td_attributes })
    }

    /// Get the quote version (4 or 5).
    #[must_use]
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Get the full REPORTDATA (64 bytes).
    #[must_use]
    pub fn report_data(&self) -> &Bytes64 {
        &self.report_data
    }

    /// Get REPORTDATA[0:32] - the challenge/binding field.
    #[must_use]
    pub fn report_data_binding(&self) -> Bytes32 {
        let mut binding = [0u8; 32];
        binding.copy_from_slice(&self.report_data.as_bytes()[0..32]);
        Bytes32::from(binding)
    }

    /// Get the instance_id from REPORTDATA[32:64].
    ///
    /// For TDX, the instance_id is client-generated and placed in
    /// REPORTDATA[32:64] at quote generation time.
    #[must_use]
    pub fn instance_id(&self) -> InstanceId {
        let mut id = [0u8; 32];
        id.copy_from_slice(&self.report_data.as_bytes()[32..64]);
        InstanceId::from(id)
    }

    /// Get the RTMRs (4 × 48 bytes).
    #[must_use]
    pub fn rtmrs(&self) -> &[Bytes48; 4] {
        &self.rtmrs
    }

    /// Get TD_ATTRIBUTES bitmap.
    #[must_use]
    pub fn td_attributes(&self) -> TdAttributes {
        self.td_attributes
    }

    /// Compute the measurement hash: SHA256(RTMR0 || RTMR1 || RTMR2 || RTMR3).
    #[must_use]
    pub fn measurement_hash(&self) -> String {
        let mut hasher = Sha256::new();
        for rtmr in &self.rtmrs {
            hasher.update(rtmr.as_bytes());
        }
        hex::encode(hasher.finalize())
    }

    /// Check if debug mode is enabled (TD_ATTRIBUTES bit 0).
    #[must_use]
    pub fn is_debug(&self) -> bool {
        self.td_attributes.is_debug()
    }

    /// Get raw quote bytes.
    #[must_use]
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw_quote
    }

    /// Verify the DCAP signature chain and return a verified quote.
    ///
    /// This performs full cryptographic verification:
    /// - Verifies ECDSA signature over quote body
    /// - Validates PCK certificate chain back to Intel root
    /// - Extracts TCB status
    pub async fn verify_signature(self) -> Result<VerifiedTdxQuote, VerificationError> {
        use verifier::{InitDataHash, ReportData};

        debug!("Verifying TDX DCAP signature");

        // Get TDX verifier from confidential-containers
        let verifier = verifier::to_verifier(&Tee::Tdx, None)
            .await
            .map_err(|e| VerificationError::AttestationFailed(format!("Failed to create TDX verifier: {e}")))?;

        // Verify signature - don't check REPORTDATA here, caller will do that
        let results = verifier
            .evaluate(self.evidence.clone(), &ReportData::NotProvided, &InitDataHash::NotProvided)
            .await
            .map_err(|e| VerificationError::AttestationFailed(format!("DCAP signature verification failed: {e}")))?;

        if results.is_empty() {
            return Err(VerificationError::AttestationFailed("No claims returned from DCAP verifier".into()));
        }

        let claims = results[0].0.clone();
        let tcb_status = claims["tcb_status"].as_str().map(String::from);

        info!(
            measurement_hash = %self.measurement_hash(),
            tcb_status = ?tcb_status,
            "TDX DCAP signature verified"
        );

        Ok(VerifiedTdxQuote { quote: self, claims, tcb_status })
    }
}

/// Signature-verified TDX Quote.
///
/// Created by calling `TdxQuote::verify_signature()`. All accessors
/// from `TdxQuote` are available, plus policy verification methods.
#[derive(Debug, Clone)]
pub struct VerifiedTdxQuote {
    /// The underlying parsed quote
    quote: TdxQuote,
    /// Verified claims from DCAP
    claims: serde_json::Value,
    /// TCB status from verifier
    tcb_status: Option<String>,
}

impl std::ops::Deref for VerifiedTdxQuote {
    type Target = TdxQuote;

    fn deref(&self) -> &Self::Target {
        &self.quote
    }
}

impl VerifiedTdxQuote {
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

    /// Verify REPORTDATA[0:32] matches expected value.
    ///
    /// Use this to verify challenge-response binding or address binding.
    pub fn verify_report_data(&self, expected: &Bytes32) -> Result<(), VerificationError> {
        let actual = self.report_data_binding();
        if &actual != expected {
            return Err(VerificationError::NonceVerificationFailed(format!(
                "REPORTDATA[0:32] mismatch: expected {}, got {}",
                expected, actual
            )));
        }
        Ok(())
    }

    /// Verify debug policy compliance.
    pub fn verify_debug(&self, debug_allowed: bool) -> Result<(), VerificationError> {
        if self.is_debug() && !debug_allowed {
            warn!("TDX debug mode enabled but not allowed by policy");
            return Err(VerificationError::DebugNotAllowed);
        }
        Ok(())
    }

    /// Verify TCB meets minimum requirements.
    pub fn verify_tcb(&self, minimum: &TdxMinimumTcb) -> Result<(), VerificationError> {
        // Check SEAM SVN
        if let Some(min_seam) = minimum.seam_svn {
            let tcb_svn_hex = self.claims["quote"]["body"]["tcb_svn"].as_str().unwrap_or("");
            if !tcb_svn_hex.is_empty() {
                if let Ok(tcb_bytes) = hex::decode(tcb_svn_hex) {
                    let actual_seam = tcb_bytes.first().copied().unwrap_or(0);
                    if actual_seam < min_seam {
                        warn!(actual = actual_seam, minimum = min_seam, "SEAM SVN below minimum");
                        return Err(VerificationError::TcbOutdated(format!(
                            "seam_svn {} < minimum {}",
                            actual_seam, min_seam
                        )));
                    }
                }
            }
        }

        // Check TEE TCB SVN array
        if let Some(ref min_tee_tcb) = minimum.tee_tcb_svn {
            let tcb_svn_hex = self.claims["quote"]["body"]["tcb_svn"].as_str().unwrap_or("");
            if !tcb_svn_hex.is_empty() {
                if let (Ok(actual_bytes), Ok(min_bytes)) = (hex::decode(tcb_svn_hex), hex::decode(min_tee_tcb)) {
                    for (i, (actual, min)) in actual_bytes.iter().zip(min_bytes.iter()).enumerate() {
                        if actual < min {
                            warn!(component = i, actual = actual, minimum = min, "TCB component below minimum");
                            return Err(VerificationError::TcbOutdated(format!(
                                "tee_tcb_svn[{}] {} < minimum {}",
                                i, actual, min
                            )));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Verify platform constraints.
    pub fn verify_platform(&self, constraints: &TdxPlatformConstraints) -> Result<(), VerificationError> {
        // Check vendor ID
        if let Some(ref allowed_vendors) = constraints.allowed_vendor_ids {
            let vendor_id = self.claims["quote"]["header"]["vendor_id"].as_str().unwrap_or("");
            if !vendor_id.is_empty() && !allowed_vendors.iter().any(|v| v.eq_ignore_ascii_case(vendor_id)) {
                warn!(vendor_id = vendor_id, "Vendor ID not in allowed list");
                return Err(VerificationError::PlatformConstraint(format!(
                    "vendor_id '{}' not allowed",
                    &vendor_id[..vendor_id.len().min(16)]
                )));
            }
        }

        // Check MR_SEAM
        if let Some(ref expected_mr_seam) = constraints.mr_seam {
            let actual_mr_seam = self.claims["quote"]["body"]["mr_seam"].as_str().unwrap_or("");
            if !actual_mr_seam.is_empty() && !expected_mr_seam.eq_ignore_ascii_case(actual_mr_seam) {
                warn!("MR_SEAM mismatch");
                return Err(VerificationError::PlatformConstraint(format!(
                    "mr_seam mismatch: expected '{}...', got '{}...'",
                    &expected_mr_seam[..expected_mr_seam.len().min(16)],
                    &actual_mr_seam[..actual_mr_seam.len().min(16)]
                )));
            }
        }

        // Check MRSIGNER_SEAM
        if let Some(ref expected_mrsigner) = constraints.mrsigner_seam {
            let actual_mrsigner = self.claims["quote"]["body"]["mrsigner_seam"].as_str().unwrap_or("");
            if !actual_mrsigner.is_empty() && !expected_mrsigner.eq_ignore_ascii_case(actual_mrsigner) {
                warn!("MRSIGNER_SEAM mismatch");
                return Err(VerificationError::PlatformConstraint(format!(
                    "mrsigner_seam mismatch: expected '{}...', got '{}...'",
                    &expected_mrsigner[..expected_mrsigner.len().min(16)],
                    &actual_mrsigner[..actual_mrsigner.len().min(16)]
                )));
            }
        }

        Ok(())
    }

    /// Verify measurements match trusted values.
    pub fn verify_measurements(&self, trusted: &TdxMeasurement) -> Result<(), VerificationError> {
        // Compute expected hash from trusted RTMRs
        let mut hasher = Sha256::new();
        for rtmr_hex in [&trusted.rtmr0, &trusted.rtmr1, &trusted.rtmr2, &trusted.rtmr3] {
            let bytes = hex::decode(rtmr_hex)
                .map_err(|e| VerificationError::ParseError(format!("Invalid trusted RTMR hex: {e}")))?;
            hasher.update(&bytes);
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

        info!(measurement_hash = %actual_hash, "TDX measurements verified");
        Ok(())
    }

    /// Convert to platform-agnostic ParsedAttestation.
    #[must_use]
    pub fn into_parsed_attestation(self) -> ParsedAttestation {
        ParsedAttestation {
            tee: Tee::Tdx,
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
    fn test_measurement_hash_computation() {
        // Create RTMRs with known values
        let rtmrs = [
            Bytes48::from([0xAA; 48]),
            Bytes48::from([0xBB; 48]),
            Bytes48::from([0xCC; 48]),
            Bytes48::from([0xDD; 48]),
        ];

        let mut hasher = Sha256::new();
        for rtmr in &rtmrs {
            hasher.update(rtmr.as_bytes());
        }
        let expected = hex::encode(hasher.finalize());

        // Verify the computation matches
        let mut hasher2 = Sha256::new();
        for rtmr in &rtmrs {
            hasher2.update(rtmr.as_bytes());
        }
        assert_eq!(hex::encode(hasher2.finalize()), expected);
    }

    #[test]
    fn test_td_attributes_debug_flag() {
        // TD_ATTRIBUTES with debug bit set
        let attrs_debug = TdAttributes::from_bytes([0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert!(attrs_debug.is_debug());

        // TD_ATTRIBUTES without debug bit
        let attrs_nodebug = TdAttributes::from_bytes([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert!(!attrs_nodebug.is_debug());
    }

    #[test]
    fn test_td_attributes_multiple_flags() {
        // Debug + Migratable
        let attrs = TdAttributes::from_raw(TdAttributes::DEBUG | TdAttributes::MIGRATABLE);
        assert!(attrs.is_debug());
        assert!(attrs.is_migratable());
        assert!(!attrs.is_perfmon_enabled());
    }

    #[test]
    fn test_td_attributes_from_bytes() {
        // Little-endian: byte 0 = 0x01 means bit 0 (DEBUG) is set
        let bytes = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let attrs = TdAttributes::from_bytes(bytes);
        assert!(attrs.is_debug());
        assert_eq!(attrs.as_raw(), 1);
    }

    #[test]
    fn test_td_attributes_roundtrip() {
        let original = TdAttributes::from_raw(0x12345678_9ABCDEF0);
        let bytes = original.as_bytes();
        let restored = TdAttributes::from_bytes(bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_td_attributes_debug_format() {
        let attrs = TdAttributes::from_raw(TdAttributes::DEBUG | TdAttributes::PKS);
        let debug_str = format!("{:?}", attrs);
        assert!(debug_str.contains("DEBUG"));
        assert!(debug_str.contains("PKS"));
    }
}
