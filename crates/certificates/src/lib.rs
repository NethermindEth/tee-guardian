// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Certificate Authority for TEE-KMS
//!
//! This crate provides certificate management for external namespaces (non-guardian).
//! It handles:
//! - Certificate issuance for TEE nodes in external namespaces
//! - Certificate revocation and tracking
//! - TDX/SEV-SNP quote certificate chain parsing and display
//!
//! ## Design Notes
//!
//! The "guardian" namespace is reserved for the Guardian cluster itself and uses
//! direct attestation-based trust (no certificates). All other namespaces use
//! certificates for identity, allowing TEE applications to verify each other
//! with standard TLS/mTLS without re-implementing attestation verification.

mod error;
mod quote_certs;
mod quote_certs_parse;
mod tee_oids;
mod types;

pub use error::{CertificateError, CertificateResult};
pub use quote_certs::{
    CertChain, CertDataType, CertInfo, QuoteSignature, QuoteSignatureError, TeeExtension, TeePlatform,
};
pub use quote_certs_parse::{parse_certificate_der, parse_certificate_pem, parse_quote_signature};
pub use tee_oids::{is_tee_oid, oid_name, tee_platform, TEE_OID_REGISTRY};
pub use types::{
    Certificate, CertificateRequest, CertificateStatus, IssuedCertificate, Namespace, NamespaceConfig, RevocationReason,
};

/// Check if a namespace name is reserved
#[must_use]
pub fn is_reserved_namespace(name: &str) -> bool {
    name == common::config::GUARDIAN_CERTIFICATE_NAMESPACE
}
