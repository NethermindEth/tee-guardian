// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Certificate chain parsing for TEE attestation quotes.
//!
//! Parses certificate chains from TDX quotes and standalone VCEK certificates.
//! Uses lenient parsing - extracts as much as possible, logs failures.

use crate::quote_certs::{CertChain, CertDataType, CertInfo, QuoteSignature, QuoteSignatureError, TeeExtension};
use crate::tee_oids::{is_tee_oid, oid_name, tee_platform, TEE_OID_REGISTRY};
use x509_parser::extensions::ParsedExtension;
use x509_parser::prelude::*;
use x509_parser::public_key::PublicKey;

/// Parse the signature data section of a TDX quote.
pub fn parse_quote_signature(
    quote_bytes: &[u8],
    sig_data_offset: usize,
) -> Result<QuoteSignature, QuoteSignatureError> {
    if quote_bytes.len() < sig_data_offset + 4 {
        return Err(QuoteSignatureError::TooShort("Quote too short for signature".into()));
    }

    let sig_data = &quote_bytes[sig_data_offset..];

    // Read signature data length
    let sig_data_len = u32::from_le_bytes([sig_data[0], sig_data[1], sig_data[2], sig_data[3]]) as usize;

    if sig_data.len() < 4 + sig_data_len {
        return Err(QuoteSignatureError::TooShort("Signature data truncated".into()));
    }

    let sig_content = &sig_data[4..4 + sig_data_len];

    // Minimum: 64B signature + 64B pubkey + 2B cert type + 4B cert size = 134 bytes
    if sig_content.len() < 134 {
        return Err(QuoteSignatureError::TooShort("Signature content too short".into()));
    }

    let signature = sig_content[..64].to_vec();
    let attest_pub_key = sig_content[64..128].to_vec();

    let cert_data_type_raw = u16::from_le_bytes([sig_content[128], sig_content[129]]);
    let cert_data_type = CertDataType::from_u16(cert_data_type_raw);

    let cert_data_size =
        u32::from_le_bytes([sig_content[130], sig_content[131], sig_content[132], sig_content[133]]) as usize;

    // Parse certificate chain if present
    let cert_chain = if sig_content.len() >= 134 + cert_data_size && cert_data_size > 0 {
        let cert_data = &sig_content[134..134 + cert_data_size];
        parse_cert_data(cert_data_type.unwrap_or(CertDataType::PckCertChain), cert_data).ok()
    } else {
        None
    };

    Ok(QuoteSignature { signature, attest_pub_key, cert_data_type, cert_chain, cert_data_size })
}

/// Parse certification data based on type
fn parse_cert_data(data_type: CertDataType, data: &[u8]) -> Result<CertChain, QuoteSignatureError> {
    match data_type {
        CertDataType::PckCertChain | CertDataType::PckLeafCert => parse_pem_chain(data_type, data),
        CertDataType::QeReportCertData => parse_qe_report_cert_data(data),
        _ => Ok(CertChain { data_type, certs: Vec::new() }),
    }
}

/// Parse QE Report certification data (Type 6)
fn parse_qe_report_cert_data(data: &[u8]) -> Result<CertChain, QuoteSignatureError> {
    let min_size = 384 + 64 + 2; // QE Report + signature + auth size
    if data.len() < min_size {
        return Err(QuoteSignatureError::CertificateError("QE Report data too short".into()));
    }

    let mut pos = 384 + 64; // Skip QE Report and signature

    let auth_size = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
    pos += 2 + auth_size;

    if data.len() < pos + 6 {
        return Err(QuoteSignatureError::CertificateError("Missing nested cert data".into()));
    }

    let nested_type_raw = u16::from_le_bytes([data[pos], data[pos + 1]]);
    let nested_type = CertDataType::from_u16(nested_type_raw).unwrap_or(CertDataType::PckCertChain);
    pos += 2;

    let nested_size = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
    pos += 4;

    if data.len() < pos + nested_size {
        return Err(QuoteSignatureError::CertificateError("Nested cert data truncated".into()));
    }

    parse_pem_chain(nested_type, &data[pos..pos + nested_size])
}

/// Parse PEM-encoded certificate chain
fn parse_pem_chain(data_type: CertDataType, pem_data: &[u8]) -> Result<CertChain, QuoteSignatureError> {
    let pem_str =
        std::str::from_utf8(pem_data).map_err(|_| QuoteSignatureError::CertificateError("Invalid UTF-8".into()))?;

    let mut certs = Vec::new();
    for pem_block in pem_str.split("-----END CERTIFICATE-----") {
        let trimmed = pem_block.trim();
        if !trimmed.contains("-----BEGIN CERTIFICATE-----") {
            continue;
        }

        let full_pem = format!("{}-----END CERTIFICATE-----", trimmed);
        if let Some(der) = pem_to_der(&full_pem) {
            if let Ok(cert_info) = parse_x509_cert(&der) {
                certs.push(cert_info);
            }
        }
    }

    Ok(CertChain { data_type, certs })
}

/// Convert PEM to DER
fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let start = pem.find("-----BEGIN CERTIFICATE-----")? + 27;
    let end = pem.find("-----END CERTIFICATE-----")?;
    let b64: String = pem[start..end].chars().filter(|c| !c.is_whitespace()).collect();
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64).ok()
}

/// Parse a DER-encoded X.509 certificate.
pub fn parse_certificate_der(der_bytes: &[u8]) -> Result<CertInfo, QuoteSignatureError> {
    parse_x509_cert(der_bytes)
}

/// Parse a PEM-encoded X.509 certificate
pub fn parse_certificate_pem(pem: &str) -> Result<CertInfo, QuoteSignatureError> {
    let der = pem_to_der(pem).ok_or_else(|| QuoteSignatureError::CertificateError("Invalid PEM".into()))?;
    parse_x509_cert(&der)
}

/// Parse an X.509 certificate from DER bytes.
fn parse_x509_cert(der_bytes: &[u8]) -> Result<CertInfo, QuoteSignatureError> {
    let (_, cert) = X509Certificate::from_der(der_bytes)
        .map_err(|e| QuoteSignatureError::CertificateError(format!("Parse error: {}", e)))?;

    let tbs = cert.tbs_certificate;

    // Extract TEE extensions
    let tee_extensions: Vec<TeeExtension> = tbs
        .extensions()
        .iter()
        .filter_map(|ext| {
            let oid_str = ext.oid.to_string();
            if !is_tee_oid(&oid_str) {
                return None;
            }

            let platform = tee_platform(&oid_str)?;
            let name = oid_name(&ext.oid);
            let value = decode_extension_value(ext.value);

            Some(TeeExtension { oid: oid_str, name, value, platform })
        })
        .collect();

    // Extract SKI
    let ski = tbs
        .extensions()
        .iter()
        .find(|e| e.oid == x509_parser::oid_registry::OID_X509_EXT_SUBJECT_KEY_IDENTIFIER)
        .and_then(|ext| match ext.parsed_extension() {
            ParsedExtension::SubjectKeyIdentifier(ski) => Some(hex::encode(ski.0)),
            _ => None,
        });

    // Extract AKI
    let aki = tbs
        .extensions()
        .iter()
        .find(|e| e.oid == x509_parser::oid_registry::OID_X509_EXT_AUTHORITY_KEY_IDENTIFIER)
        .and_then(|ext| match ext.parsed_extension() {
            ParsedExtension::AuthorityKeyIdentifier(aki) => aki.key_identifier.as_ref().map(|ki| hex::encode(ki.0)),
            _ => None,
        });

    // Extract BasicConstraints
    let is_ca = tbs
        .extensions()
        .iter()
        .find(|e| e.oid == x509_parser::oid_registry::OID_X509_EXT_BASIC_CONSTRAINTS)
        .and_then(|ext| match ext.parsed_extension() {
            ParsedExtension::BasicConstraints(bc) => Some(bc.ca),
            _ => None,
        })
        .unwrap_or(false);

    Ok(CertInfo {
        subject_cn: extract_cn(tbs.subject()),
        issuer_cn: extract_cn(tbs.issuer()),
        subject_dn: tbs.subject().to_string(),
        issuer_dn: tbs.issuer().to_string(),
        serial: hex::encode(tbs.raw_serial()),
        not_before: tbs.validity().not_before.to_string(),
        not_after: tbs.validity().not_after.to_string(),
        sig_alg: TEE_OID_REGISTRY
            .get(cert.signature_algorithm.oid())
            .map(|e| e.sn().to_string())
            .unwrap_or_else(|| cert.signature_algorithm.oid().to_string()),
        key_alg: TEE_OID_REGISTRY
            .get(tbs.subject_pki.algorithm.oid())
            .map(|e| e.sn().to_string())
            .unwrap_or_else(|| tbs.subject_pki.algorithm.oid().to_string()),
        key_bits: tbs
            .subject_pki
            .parsed()
            .map(|pk| match pk {
                PublicKey::RSA(rsa) => rsa.key_size(),
                PublicKey::EC(ec) => ec.data().len() * 8,
                _ => 0,
            })
            .unwrap_or(0),
        ski,
        aki,
        is_ca,
        tee_extensions,
        der_size: der_bytes.len(),
    })
}

/// Extract Common Name from X.500 name
fn extract_cn(name: &X509Name) -> String {
    name.iter_common_name().next().and_then(|cn| cn.as_str().ok()).unwrap_or("Unknown").to_string()
}

/// Decode extension value to string (hex for binary, try to decode ASN.1 integers/strings)
fn decode_extension_value(value: &[u8]) -> String {
    // Try INTEGER (tag 0x02)
    if value.len() >= 2 && value[0] == 0x02 {
        let len = value[1] as usize;
        if value.len() >= 2 + len && len <= 8 {
            let mut n: u64 = 0;
            for &b in &value[2..2 + len] {
                n = n.wrapping_shl(8) | u64::from(b);
            }
            return n.to_string();
        }
    }

    // Try OCTET STRING (tag 0x04)
    if value.len() >= 2 && value[0] == 0x04 {
        let len = value[1] as usize;
        if value.len() >= 2 + len {
            return hex::encode(&value[2..2 + len]);
        }
    }

    // Try UTF8String (tag 0x0C), PrintableString (tag 0x13), IA5String (tag 0x16)
    if value.len() >= 2 && matches!(value[0], 0x0C | 0x13 | 0x16) {
        let len = value[1] as usize;
        if value.len() >= 2 + len {
            if let Ok(s) = std::str::from_utf8(&value[2..2 + len]) {
                return s.to_string();
            }
        }
    }

    // Fallback: hex encode
    hex::encode(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quote_certs::TeePlatform;

    // AMD VCEK certificate for testing
    const AMD_VCEK_DER: &[u8] = include_bytes!("../test_data/amd_vcek.der");

    #[test]
    fn test_parse_amd_vcek() {
        let cert = parse_certificate_der(AMD_VCEK_DER).expect("Failed to parse AMD VCEK");

        assert_eq!(cert.subject_cn, "SEV-VCEK");
        assert!(!cert.tee_extensions.is_empty());

        // All extensions should be AMD
        for ext in &cert.tee_extensions {
            assert_eq!(ext.platform, TeePlatform::Amd);
        }

        // Should have Product extension containing "Milan"
        let product = cert.tee_extensions.iter().find(|e| e.name == "Product");
        assert!(product.is_some());
        assert!(product.unwrap().value.contains("Milan"));
    }

    #[test]
    fn test_amd_vcek_display() {
        let cert = parse_certificate_der(AMD_VCEK_DER).expect("Failed to parse AMD VCEK");
        let display = format!("{}", cert);
        assert!(display.contains("SEV-VCEK"));
        assert!(display.contains("TEE Extensions"));
    }
}
