// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! TEE quote certificate types and display formatting.
//!
//! Types representing the certificate chain embedded in TDX quotes and
//! VCEK certificates for SEV-SNP, used for attestation verification.
//!
//! Supports both:
//! - **Intel SGX/TDX**: PCK certificates with extensions under OID 1.2.840.113741.1.13.*
//! - **AMD SEV-SNP**: VCEK certificates with extensions under OID 1.3.6.1.4.1.3704.1.*

use std::fmt;

/// Certification data types in DCAP quotes
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertDataType {
    /// PPID (Platform Provisioning ID) in plain text
    PpidPlaintext = 1,
    /// PPID encrypted with RSA-2048
    PpidRsa2048 = 2,
    /// PPID encrypted with RSA-3072
    PpidRsa3072 = 3,
    /// PCK leaf certificate only
    PckLeafCert = 4,
    /// Full certificate chain in PEM format
    PckCertChain = 5,
    /// QE Report certification data (contains nested cert chain)
    QeReportCertData = 6,
    /// Platform manifest
    PlatformManifest = 7,
}

impl CertDataType {
    /// Parse from 2-byte little-endian value
    #[must_use]
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::PpidPlaintext),
            2 => Some(Self::PpidRsa2048),
            3 => Some(Self::PpidRsa3072),
            4 => Some(Self::PckLeafCert),
            5 => Some(Self::PckCertChain),
            6 => Some(Self::QeReportCertData),
            7 => Some(Self::PlatformManifest),
            _ => None,
        }
    }
}

impl fmt::Display for CertDataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PpidPlaintext => write!(f, "PPID Plaintext"),
            Self::PpidRsa2048 => write!(f, "PPID RSA-2048"),
            Self::PpidRsa3072 => write!(f, "PPID RSA-3072"),
            Self::PckLeafCert => write!(f, "PCK Leaf Cert"),
            Self::PckCertChain => write!(f, "PCK Cert Chain"),
            Self::QeReportCertData => write!(f, "QE Report + Certs"),
            Self::PlatformManifest => write!(f, "Platform Manifest"),
        }
    }
}

/// Parsed certificate with extracted metadata
#[derive(Debug, Clone)]
pub struct CertInfo {
    /// Certificate subject common name
    pub subject_cn: String,
    /// Certificate issuer common name
    pub issuer_cn: String,
    /// Full subject distinguished name
    pub subject_dn: String,
    /// Full issuer distinguished name
    pub issuer_dn: String,
    /// Serial number (hex)
    pub serial: String,
    /// Not valid before
    pub not_before: String,
    /// Not valid after
    pub not_after: String,
    /// Signature algorithm
    pub sig_alg: String,
    /// Public key algorithm
    pub key_alg: String,
    /// Public key size in bits
    pub key_bits: usize,
    /// Subject Key Identifier (hex), if present
    pub ski: Option<String>,
    /// Authority Key Identifier (hex), if present
    pub aki: Option<String>,
    /// Is CA certificate
    pub is_ca: bool,
    /// TEE-specific extensions (Intel SGX/TDX or AMD SEV-SNP)
    pub tee_extensions: Vec<TeeExtension>,
    /// Raw DER size
    pub der_size: usize,
}

impl fmt::Display for CertInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Subject:  {}", self.subject_dn)?;
        writeln!(f, "Issuer:   {}", self.issuer_dn)?;
        writeln!(f, "Serial:   {}", &self.serial[..self.serial.len().min(40)])?;
        writeln!(f, "Valid:    {} to {}", self.not_before, self.not_after)?;
        writeln!(f, "Key:      {} {} bits, {}", self.key_alg, self.key_bits, self.sig_alg)?;
        if !self.tee_extensions.is_empty() {
            writeln!(f, "TEE Extensions:")?;
            for ext in &self.tee_extensions {
                writeln!(f, "  [{}] {}:  {}", ext.platform, ext.name, ext.value)?;
            }
        }
        Ok(())
    }
}

/// TEE platform identifier for certificate extensions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeePlatform {
    /// Intel SGX/TDX (PCK certificate)
    Intel,
    /// AMD SEV-SNP (VCEK certificate)
    Amd,
}

impl fmt::Display for TeePlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Intel => write!(f, "Intel"),
            Self::Amd => write!(f, "AMD"),
        }
    }
}

/// TEE-specific X.509 certificate extension.
///
/// Covers both Intel SGX/TDX (PCK certificate) and AMD SEV-SNP (VCEK certificate) OIDs.
///
/// # Intel OIDs (PCK Certificate)
///
/// | OID | Field | Size | Description |
/// |-----|-------|------|-------------|
/// | 1.2.840.113741.1.13.1.1 | PPID | 16B | Platform Provisioning ID |
/// | 1.2.840.113741.1.13.1.2 | TCB | var | TCB component versions |
/// | 1.2.840.113741.1.13.1.2.17 | PCESVN | 2B | PCE security version |
/// | 1.2.840.113741.1.13.1.2.18 | CPUSVN | 16B | CPU security version |
/// | 1.2.840.113741.1.13.1.3 | PCE-ID | 2B | PCE identifier |
/// | 1.2.840.113741.1.13.1.4 | FMSPC | 6B | Family-Model-Stepping |
/// | 1.2.840.113741.1.13.1.5 | SGX Type | var | Platform type |
/// | 1.2.840.113741.1.13.1.6 | Platform Instance ID | var | Platform instance |
///
/// # AMD OIDs (VCEK Certificate)
///
/// | OID | Field | Description |
/// |-----|-------|-------------|
/// | 1.3.6.1.4.1.3704.1.1 | BlSpl | Bootloader Security Patch Level |
/// | 1.3.6.1.4.1.3704.1.2 | TeeSpl | TEE Security Patch Level |
/// | 1.3.6.1.4.1.3704.1.3 | SnpSpl | SNP Firmware Security Patch Level |
/// | 1.3.6.1.4.1.3704.1.4 | UcodeSpl | Microcode Security Patch Level |
#[derive(Debug, Clone)]
pub struct TeeExtension {
    /// OID string (e.g., "1.2.840.113741.1.13.1.1")
    pub oid: String,
    /// Human-readable field name (e.g., "PPID")
    pub name: String,
    /// Decoded value (hex-encoded for binary fields, decimal for integers)
    pub value: String,
    /// TEE platform this extension belongs to
    pub platform: TeePlatform,
}

impl fmt::Display for TeeExtension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.name, self.oid, self.value)
    }
}

/// Parsed certificate chain from a TDX quote
#[derive(Debug, Clone)]
pub struct CertChain {
    /// Certification data type
    pub data_type: CertDataType,
    /// Certificates in chain order (leaf first, root last)
    pub certs: Vec<CertInfo>,
}

/// ECDSA signature data from quote
#[derive(Debug, Clone)]
pub struct QuoteSignature {
    /// ECDSA signature (r || s, 64 bytes for P-256)
    pub signature: Vec<u8>,
    /// Attestation public key (x || y, 64 bytes)
    pub attest_pub_key: Vec<u8>,
    /// Certification data type
    pub cert_data_type: Option<CertDataType>,
    /// Certificate chain (if parseable)
    pub cert_chain: Option<CertChain>,
    /// Raw certification data size
    pub cert_data_size: usize,
}

/// Error type for quote signature parsing
#[derive(Debug, thiserror::Error)]
pub enum QuoteSignatureError {
    /// Quote is too short
    #[error("Quote too short: {0}")]
    TooShort(String),
    /// Invalid signature data
    #[error("Invalid signature data: {0}")]
    InvalidSignatureData(String),
    /// Certificate parsing error
    #[error("Certificate parsing error: {0}")]
    CertificateError(String),
}
