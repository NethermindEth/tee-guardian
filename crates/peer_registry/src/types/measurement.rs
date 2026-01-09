// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Trusted measurement types from the registry.

use serde::Deserialize;

/// Trusted measurement from the registry with constraint requirements.
#[derive(Debug, Clone, Deserialize)]
pub struct TrustedMeasurement {
    /// Namespace this measurement belongs to
    pub namespace: String,
    /// Platform type
    pub platform: String,
    /// Platform-specific data
    pub data: TrustedMeasurementData,
    /// Revocation timestamp (None if not revoked)
    pub revoked_at: Option<u64>,
}

/// Platform-specific trusted measurement data.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TrustedMeasurementData {
    /// TDX measurement data
    Tdx(TdxMeasurement),
    /// SEV-SNP measurement data
    Snp(SnpMeasurement),
}

/// TDX-specific measurement constraints.
#[derive(Debug, Clone, Deserialize)]
pub struct TdxMeasurement {
    /// RTMR0 (48 bytes, hex)
    pub rtmr0: String,
    /// RTMR1 (48 bytes, hex)
    pub rtmr1: String,
    /// RTMR2 (48 bytes, hex)
    pub rtmr2: String,
    /// RTMR3 (48 bytes, hex)
    pub rtmr3: String,
    /// Allow debug TDs (default: false)
    #[serde(default)]
    pub debug_allowed: bool,
    /// Minimum TCB requirements
    pub minimum_tcb: Option<TdxMinimumTcb>,
    /// Platform constraints
    pub platform_constraints: Option<TdxPlatformConstraints>,
}

/// TDX minimum TCB requirements.
#[derive(Debug, Clone, Deserialize)]
pub struct TdxMinimumTcb {
    /// Minimum SEAM SVN
    pub seam_svn: Option<u8>,
    /// Minimum TEE TCB SVN (16 bytes, hex)
    pub tee_tcb_svn: Option<String>,
}

/// TDX platform constraints.
#[derive(Debug, Clone, Deserialize)]
pub struct TdxPlatformConstraints {
    /// Allowed vendor IDs (hex)
    pub allowed_vendor_ids: Option<Vec<String>>,
    /// Required MR_SEAM (hex)
    pub mr_seam: Option<String>,
    /// Required MRSIGNER_SEAM (hex)
    pub mrsigner_seam: Option<String>,
    /// Allowed CPU models
    pub allowed_cpu_models: Option<Vec<CpuModel>>,
}

/// SEV-SNP-specific measurement constraints.
#[derive(Debug, Clone, Deserialize)]
pub struct SnpMeasurement {
    /// Launch measurement (48 bytes, hex)
    pub measurement: String,
    /// Host data (32 bytes, hex) - optional
    pub host_data: Option<String>,
    /// Allow debug guests (default: false)
    #[serde(default)]
    pub debug_allowed: bool,
    /// Minimum TCB requirements
    pub minimum_tcb: Option<SnpMinimumTcb>,
    /// Platform constraints
    pub platform_constraints: Option<SnpPlatformConstraints>,
}

/// SEV-SNP minimum TCB requirements.
#[derive(Debug, Clone, Deserialize)]
pub struct SnpMinimumTcb {
    /// Minimum bootloader SVN
    pub bootloader_svn: Option<u8>,
    /// Minimum TEE SVN
    pub tee_svn: Option<u8>,
    /// Minimum SNP firmware SVN
    pub snp_svn: Option<u8>,
    /// Minimum microcode SVN
    pub microcode_svn: Option<u8>,
}

/// SEV-SNP platform constraints.
#[derive(Debug, Clone, Deserialize)]
pub struct SnpPlatformConstraints {
    /// Allowed chip IDs (64 bytes each, hex)
    pub allowed_chip_ids: Option<Vec<String>>,
    /// Allowed CPU models
    pub allowed_cpu_models: Option<Vec<CpuModel>>,
    /// Require SMT disabled
    pub require_smt_disabled: Option<bool>,
    /// Required VMPL level
    pub required_vmpl: Option<u8>,
    /// Required ID key digest (hex)
    pub id_key_digest: Option<String>,
    /// Required author key digest (hex)
    pub author_key_digest: Option<String>,
}

/// CPU model specification.
#[derive(Debug, Clone, Deserialize)]
pub struct CpuModel {
    /// CPU family
    pub family: u8,
    /// CPU model
    pub model: u8,
    /// CPU stepping (None matches any)
    pub stepping: Option<u8>,
}
