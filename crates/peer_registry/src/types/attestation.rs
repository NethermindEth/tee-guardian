// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Parsed attestation types.

use super::identity::InstanceId;
use kbs_types::Tee;
use std::fmt;

/// Platform-agnostic parsed attestation result.
///
/// Contains the essential fields extracted from any TEE attestation,
/// normalized into a common format.
#[derive(Debug, Clone)]
pub struct ParsedAttestation {
    /// TEE platform type (TDX, SNP, etc.)
    pub tee: Tee,
    /// Unique instance identifier
    /// - TDX: REPORTDATA[32:64] (client-generated)
    /// - SEV-SNP: REPORT_ID (firmware-generated)
    pub instance_id: InstanceId,
    /// Measurement hash identifying the software stack
    /// - TDX: SHA256(RTMR0 || RTMR1 || RTMR2 || RTMR3)
    /// - SEV-SNP: SHA256(measurement || host_data) or SHA256(measurement)
    pub measurement_hash: String,
    /// Raw claims from the verifier (platform-specific JSON)
    pub claims: serde_json::Value,
    /// TCB status string (if available)
    pub tcb_status: Option<String>,
}

impl fmt::Display for ParsedAttestation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ParsedAttestation {{ tee: {:?}, instance_id: {}, measurement: {}... }}",
            self.tee,
            self.instance_id,
            &self.measurement_hash[..self.measurement_hash.len().min(16)]
        )
    }
}
