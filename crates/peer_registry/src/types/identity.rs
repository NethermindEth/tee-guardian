// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! TEE instance identity types.
//!
//! Unique instance identifiers extracted from attestation evidence:
//! - **TDX**: Client-provided random bytes in `REPORTDATA[32:64]`
//! - **SEV-SNP**: Firmware-generated `REPORT_ID` (32 bytes)

use super::error::VerificationError;
use crate::backend::{generate_evidence, SnpReport};
use common::Bytes32;
use kbs_types::Tee;
use std::fmt;

/// Unique instance identity (32 bytes).
///
/// For TDX, this is extracted from `REPORTDATA[32:64]`.
/// For SEV-SNP, this is the `REPORT_ID` field.
///
/// # Design
///
/// `InstanceId` wraps `Bytes32` but intentionally does NOT implement
/// `Into<Bytes32>` - this is a one-way conversion to prevent accidentally
/// treating semantic identity as generic bytes. Use `as_bytes()` if you
/// need raw byte access.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstanceId(Bytes32);

impl InstanceId {
    /// Create from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Bytes32::from(bytes))
    }

    /// Parse from hex string.
    ///
    /// # Errors
    ///
    /// Returns an error if the hex string is invalid or wrong length.
    pub fn from_hex(hex: &str) -> Result<Self, hex::FromHexError> {
        Bytes32::from_hex(hex).map(Self)
    }

    /// Check if all bytes are zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.as_bytes() == &[0u8; 32]
    }

    /// Get as byte slice.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }

    /// Convert to hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// Short hex for logging (first 8 hex chars / 4 bytes).
    #[must_use]
    pub fn short(&self) -> String {
        hex::encode(&self.0.as_bytes()[..4])
    }

    /// Load instance ID based on TEE type.
    ///
    /// - **TDX**: Reads hex from file at `instance_id_path` (generated at boot)
    /// - **SNP**: Extracts from `REPORT_ID` attestation field
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - TDX: File cannot be read, or hex is invalid/wrong length
    /// - SNP: Attestation generation fails, or report cannot be parsed
    pub async fn load_for_tee(tee: Tee, instance_id_path: &str) -> Result<Self, VerificationError> {
        match tee {
            Tee::Tdx => {
                let hex_id = std::fs::read_to_string(instance_id_path)
                    .map_err(|e| {
                        VerificationError::AttestationFailed(format!(
                            "Failed to read instance_id from {}: {}",
                            instance_id_path, e
                        ))
                    })?
                    .trim()
                    .to_string();

                if hex_id.len() != 64 {
                    return Err(VerificationError::AttestationFailed(format!(
                        "Invalid instance_id length: {} (expected 64)",
                        hex_id.len()
                    )));
                }

                Self::from_hex(&hex_id)
                    .map_err(|e| VerificationError::AttestationFailed(format!("Invalid instance_id hex: {e}")))
            }
            Tee::Snp => {
                let evidence = generate_evidence(Tee::Snp, &[0u8; 64]).await?;
                let report = SnpReport::from_json(evidence)?;
                Ok(report.instance_id())
            }
            _ => Err(VerificationError::InvalidQuoteFormat(format!("Unsupported TEE: {tee:?}"))),
        }
    }
}

// === One-way conversions INTO InstanceId ===

impl From<Bytes32> for InstanceId {
    fn from(bytes: Bytes32) -> Self {
        Self(bytes)
    }
}

impl From<[u8; 32]> for InstanceId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(Bytes32::from(bytes))
    }
}

// Note: We intentionally do NOT implement From<InstanceId> for Bytes32
// to prevent accidental conversion from semantic type to generic bytes.

impl fmt::Debug for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InstanceId({})", self.short())
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.short())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_id_from_bytes32() {
        let bytes = Bytes32::from([0xAB; 32]);
        let id = InstanceId::from(bytes);
        assert_eq!(id.as_bytes(), &[0xAB; 32]);
    }

    #[test]
    fn test_instance_id_from_array() {
        let bytes = [0xCD; 32];
        let id = InstanceId::from(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn test_instance_id_is_zero() {
        let zero = InstanceId::from([0u8; 32]);
        assert!(zero.is_zero());

        let nonzero = InstanceId::from([1u8; 32]);
        assert!(!nonzero.is_zero());
    }

    #[test]
    fn test_instance_id_hex_roundtrip() {
        let original = InstanceId::from([0xDE; 32]);
        let hex = original.to_hex();

        // Parse back via Bytes32
        let bytes = Bytes32::from_hex(&hex).unwrap();
        let restored = InstanceId::from(bytes);

        assert_eq!(original, restored);
    }

    #[test]
    fn test_instance_id_short() {
        let id = InstanceId::from([0xAB; 32]);
        assert_eq!(id.short(), "abababab");
    }

    #[test]
    fn test_instance_id_hash_eq() {
        use std::collections::HashSet;

        let id1 = InstanceId::from([1u8; 32]);
        let id2 = InstanceId::from([1u8; 32]);
        let id3 = InstanceId::from([2u8; 32]);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let mut set = HashSet::new();
        set.insert(id1);
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }
}
