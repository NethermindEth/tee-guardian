// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Certificate types
//!
//! Type definitions for certificates, namespaces, and related structures.
//! These are stub implementations that will be expanded when the full CA is implemented.

use serde::{Deserialize, Serialize};

/// Namespace configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Namespace {
    /// Unique namespace identifier (e.g., "company-k8s-prod")
    pub name: String,

    /// Namespace configuration
    pub config: NamespaceConfig,

    /// Creation timestamp (Unix seconds)
    pub created_at: u64,

    /// Last update timestamp (Unix seconds)
    pub updated_at: u64,
}

/// Configuration for a namespace
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespaceConfig {
    /// URL to the measurement registry for this namespace
    /// If None, uses the default guardian measurement registry
    pub measurement_registry_url: Option<String>,

    /// Certificate validity period in days
    pub cert_validity_days: u32,

    /// Maximum number of active certificates allowed
    pub max_active_certs: u32,

    /// Description of this namespace
    pub description: Option<String>,
}

impl Default for NamespaceConfig {
    fn default() -> Self {
        Self { measurement_registry_url: None, cert_validity_days: 365, max_active_certs: 100, description: None }
    }
}

/// Certificate request from a TEE node
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CertificateRequest {
    /// Namespace to issue certificate for
    pub namespace: String,

    /// Node's measurement ID (hash of RTMRs)
    pub measurement_id: [u8; 32],

    /// TDX attestation quote (base64 encoded)
    pub attestation_quote: String,

    /// Nonce used in attestation (for replay protection)
    pub nonce: Vec<u8>,

    /// Optional common name for the certificate
    pub common_name: Option<String>,
}

/// Issued certificate record
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuedCertificate {
    /// Unique serial number
    pub serial: u64,

    /// Namespace this certificate belongs to
    pub namespace: String,

    /// Measurement ID of the node this certificate was issued to
    pub measurement_id: [u8; 32],

    /// Common name in the certificate
    pub common_name: String,

    /// Certificate not valid before (Unix timestamp)
    pub not_before: u64,

    /// Certificate not valid after (Unix timestamp)
    pub not_after: u64,

    /// When this certificate was issued (Unix timestamp)
    pub issued_at: u64,

    /// Certificate status
    pub status: CertificateStatus,

    /// DER-encoded certificate bytes (stub - will be X.509)
    pub certificate_der: Vec<u8>,
}

/// Certificate status
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CertificateStatus {
    /// Certificate is active and valid
    Active,

    /// Certificate has been revoked
    Revoked {
        /// When the certificate was revoked (Unix timestamp)
        revoked_at: u64,
        /// Reason for revocation
        reason: RevocationReason,
    },

    /// Certificate has expired
    Expired,
}

/// Reasons for certificate revocation
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RevocationReason {
    /// Unspecified reason
    Unspecified,

    /// Key has been compromised
    KeyCompromise,

    /// CA has been compromised
    CaCompromise,

    /// Affiliation changed (node moved to different namespace)
    AffiliationChanged,

    /// Certificate has been superseded by a new one
    Superseded,

    /// Certificate is no longer needed
    CessationOfOperation,

    /// Certificate was put on hold
    CertificateHold,

    /// Measurement was revoked from registry
    MeasurementRevoked,

    /// Privilege has been withdrawn
    PrivilegeWithdrawn,
}

/// A certificate for API responses (simplified view)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Certificate {
    /// Serial number
    pub serial: u64,

    /// Namespace
    pub namespace: String,

    /// Measurement ID (hex encoded)
    pub measurement_id: String,

    /// Common name
    pub common_name: String,

    /// Valid from (Unix timestamp)
    pub not_before: u64,

    /// Valid until (Unix timestamp)
    pub not_after: u64,

    /// Status
    pub status: String,

    /// Revocation info if revoked
    pub revoked_at: Option<u64>,
    pub revocation_reason: Option<String>,
}

impl From<&IssuedCertificate> for Certificate {
    fn from(cert: &IssuedCertificate) -> Self {
        let (status, revoked_at, revocation_reason) = match &cert.status {
            CertificateStatus::Active => ("active".to_string(), None, None),
            CertificateStatus::Expired => ("expired".to_string(), None, None),
            CertificateStatus::Revoked { revoked_at, reason } => {
                ("revoked".to_string(), Some(*revoked_at), Some(format!("{:?}", reason)))
            }
        };

        Self {
            serial: cert.serial,
            namespace: cert.namespace.clone(),
            measurement_id: hex::encode(cert.measurement_id),
            common_name: cert.common_name.clone(),
            not_before: cert.not_before,
            not_after: cert.not_after,
            status,
            revoked_at,
            revocation_reason,
        }
    }
}
