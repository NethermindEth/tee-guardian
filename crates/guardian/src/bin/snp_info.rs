// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

// Copyright (c) 2024 Nethermind
//
// SPDX-License-Identifier: Apache-2.0

//! SNP Information Tool
//!
//! Displays AMD SEV-SNP attestation information by:
//! 1. Generating an attestation report via the attester crate
//! 2. Parsing and verifying it via the verifier crate to get JSON claims
//! 3. Parsing VCEK/VLEK certificates via the certificates crate

use attester::{snp::SnpAttester, Attester};
use certificates::{parse_certificate_pem, CertInfo};
use std::path::Path;
use std::process;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Check SNP availability
    let snp_device = Path::new("/dev/sev-guest").exists();

    if !snp_device {
        return Err("SNP not available: /dev/sev-guest missing".into());
    }

    // Generate attestation using attester crate
    let rt = tokio::runtime::Runtime::new()?;

    let (evidence, claims) = rt.block_on(async {
        // 1. Get raw evidence from attester
        let attester = SnpAttester::default();
        let evidence =
            attester.get_evidence(vec![0u8; 64]).await.map_err(|e| format!("Failed to get evidence: {e}"))?;

        // 2. Parse and verify via verifier to get claims
        use kbs_types::Tee;
        use verifier::{InitDataHash, ReportData};

        let snp_verifier =
            verifier::to_verifier(&Tee::Snp, None).await.map_err(|e| format!("Failed to create verifier: {e}"))?;

        let claims_result = snp_verifier
            .evaluate(evidence.clone(), &ReportData::NotProvided, &InitDataHash::NotProvided)
            .await
            .map_err(|e| format!("Verification failed: {e}"))?;

        let claims = if !claims_result.is_empty() {
            claims_result.into_iter().next().unwrap().0
        } else {
            serde_json::Value::Null
        };

        Ok::<_, String>((evidence, claims))
    })?;

    // Print the report
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                          SNP ATTESTATION REPORT                              ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    // Print TCB status
    print_tcb_status(&claims);

    // Print report header
    print_report_header(&evidence);

    // Print measurement info
    print_measurement(&evidence);

    // Print policy flags
    print_policy(&evidence, &claims);

    // Print certificate chain
    println!();
    println!("┌──────────────────────────────────────────────────────────────────────────────┐");
    println!("│                            CERTIFICATE CHAIN                                 │");
    println!("└──────────────────────────────────────────────────────────────────────────────┘");

    print_cert_chain(&evidence);

    // Raw report summary
    print_raw_report(&evidence);

    Ok(())
}

/// Print TCB status from verified claims.
fn print_tcb_status(claims: &serde_json::Value) {
    let bootloader = claims["reported_tcb_bootloader"].as_u64().unwrap_or(0);
    let tee = claims["reported_tcb_tee"].as_u64().unwrap_or(0);
    let snp = claims["reported_tcb_snp"].as_u64().unwrap_or(0);
    let microcode = claims["reported_tcb_microcode"].as_u64().unwrap_or(0);

    println!("tcb_status:     bootloader={}, tee={}, snp={}, microcode={}", bootloader, tee, snp, microcode);
}

/// Print report header fields from evidence.
fn print_report_header(evidence: &serde_json::Value) {
    let report = &evidence["attestation_report"];
    if report.is_null() {
        println!("(attestation_report not found in evidence)");
        return;
    }

    println!();
    println!("── Report Header ──");
    print_field(report, "version", "version");
    print_field(report, "guest_svn", "guest_svn");
    print_hex_field(report, "policy", "policy");
    print_hex_field(report, "family_id", "family_id");
    print_hex_field(report, "image_id", "image_id");
    print_field(report, "vmpl", "vmpl");
    print_field(report, "signature_algo", "sig_algo");
    print_hex_field(report, "platform_version", "platform_tcb");
    print_hex_field(report, "platform_info", "platform_info");
}

/// Print measurement fields.
fn print_measurement(evidence: &serde_json::Value) {
    let report = &evidence["attestation_report"];
    if report.is_null() {
        return;
    }

    println!();
    println!("── Measurement ──");
    print_hex_field(report, "measurement", "measurement");
    print_hex_field(report, "host_data", "host_data");

    // Compute measurement hash
    if let (Some(measurement), Some(host_data)) =
        (extract_bytes(report, "measurement"), extract_bytes(report, "host_data"))
    {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&measurement);

        // Include host_data only if non-zero
        if host_data.iter().any(|&b| b != 0) {
            hasher.update(&host_data);
        }

        let hash = hex::encode(hasher.finalize());
        println!("{:14} {}", "meas_hash:", hash);
    }

    println!();
    println!("── Identity ──");
    print_hex_field(report, "report_id", "report_id");
    print_hex_field(report, "report_id_ma", "report_id_ma");
    print_hex_field(report, "report_data", "report_data");
    print_hex_field(report, "id_key_digest", "id_key_digest");
    print_hex_field(report, "author_key_digest", "author_key");
    print_hex_field(report, "chip_id", "chip_id");
}

/// Print policy flags with full breakdown.
fn print_policy(evidence: &serde_json::Value, claims: &serde_json::Value) {
    let report = &evidence["attestation_report"];
    if report.is_null() {
        return;
    }

    println!();
    println!("── Policy Flags ──");

    // Extract policy value
    let policy = extract_u64(report, "policy").unwrap_or(0);

    // Bit 0: Minor version (must be 1)
    // Bit 1-3: Reserved
    // Bit 4: SMT allowed
    // Bit 5: Reserved (must be 0)
    // Bit 6-7: Reserved
    // Bit 8-15: ABI major version
    // Bit 16: Migration agent allowed
    // Bit 17: Debug allowed
    // Bit 18: Single socket required
    // Bit 19: Debug mode enabled (from claims)

    let smt_allowed = (policy >> 4) & 1 != 0;
    let migrate_ma = (policy >> 16) & 1 != 0;
    let debug_allowed = (policy >> 17) & 1 != 0;
    let single_socket = (policy >> 18) & 1 != 0;
    let abi_major = (policy >> 8) & 0xFF;
    let abi_minor = policy & 0xF;

    // Debug flag from bit 19 (actual debug state)
    let debug_enabled = (policy >> 19) & 1 != 0;

    println!("{:14} {}", "DEBUG:", debug_enabled);
    println!("{:14} {}", "DEBUG_ALLOWED:", debug_allowed);
    println!("{:14} {}", "SMT_ALLOWED:", smt_allowed);
    println!("{:14} {}", "MIGRATE_MA:", migrate_ma);
    println!("{:14} {}", "SINGLE_SOCKET:", single_socket);
    println!("{:14} {}.{}", "ABI_VERSION:", abi_major, abi_minor);

    // Additional platform info from claims
    if let Some(smt_enabled) = claims.get("platform_smt_enabled").and_then(|v| v.as_bool()) {
        println!("{:14} {}", "SMT_ENABLED:", smt_enabled);
    }

    // VMPL from report
    if let Some(vmpl) = report.get("vmpl").and_then(|v| v.as_u64()) {
        println!("{:14} {}", "VMPL:", vmpl);
    }
}

/// Print certificate chain from evidence.
fn print_cert_chain(evidence: &serde_json::Value) {
    let cert_chain = &evidence["cert_chain"];

    if cert_chain.is_null() {
        println!("(no certificate chain in evidence)");
        return;
    }

    // cert_chain can be a string (PEM) or array of PEM strings
    let certs: Vec<String> = if let Some(chain_str) = cert_chain.as_str() {
        // Single PEM string containing multiple certificates
        split_pem_chain(chain_str)
    } else if let Some(chain_arr) = cert_chain.as_array() {
        // Array of PEM strings
        chain_arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
    } else {
        println!("(unrecognized cert_chain format)");
        return;
    };

    if certs.is_empty() {
        println!("(no certificates found)");
        return;
    }

    for (i, pem) in certs.iter().enumerate() {
        let label = match i {
            0 => "VCEK (Leaf)",
            _ if i == certs.len() - 1 => "Root CA",
            _ => "Intermediate",
        };

        println!("── {} {} ──", i + 1, label);

        match parse_certificate_pem(pem) {
            Ok(cert_info) => {
                print_cert_info(&cert_info);
            }
            Err(e) => {
                println!("  Parse error: {}", e);
                // Show first few chars of PEM for debugging
                let preview: String = pem.chars().take(60).collect();
                println!("  PEM preview: {}...", preview);
            }
        }
        println!();
    }
}

/// Split a PEM string containing multiple certificates.
fn split_pem_chain(pem_data: &str) -> Vec<String> {
    let mut certs = Vec::new();

    for block in pem_data.split("-----END CERTIFICATE-----") {
        let trimmed = block.trim();
        if trimmed.contains("-----BEGIN CERTIFICATE-----") {
            certs.push(format!("{}-----END CERTIFICATE-----", trimmed));
        }
    }

    certs
}

/// Print certificate info in a readable format.
fn print_cert_info(cert: &CertInfo) {
    println!("{:14} {}", "subject:", cert.subject_cn);
    println!("{:14} {}", "issuer:", cert.issuer_cn);
    println!("{:14} {}", "serial:", &cert.serial[..32.min(cert.serial.len())]);
    println!("{:14} {} to {}", "validity:", cert.not_before, cert.not_after);
    println!("{:14} {} ({} bits)", "key:", cert.key_alg, cert.key_bits);
    println!("{:14} {}", "sig_alg:", cert.sig_alg);
    println!("{:14} {}", "is_ca:", cert.is_ca);
    println!("{:14} {} bytes", "size:", cert.der_size);

    if let Some(ref ski) = cert.ski {
        println!("{:14} {}", "SKI:", &ski[..32.min(ski.len())]);
    }
    if let Some(ref aki) = cert.aki {
        println!("{:14} {}", "AKI:", &aki[..32.min(aki.len())]);
    }

    // Print TEE-specific extensions (AMD VCEK extensions)
    if !cert.tee_extensions.is_empty() {
        println!();
        println!("  TEE Extensions:");
        for ext in &cert.tee_extensions {
            // Truncate long values
            let value_display =
                if ext.value.len() > 40 { format!("{}...", &ext.value[..40]) } else { ext.value.clone() };
            println!("    {:12} {} ({})", ext.name, value_display, ext.platform);
        }
    }
}

/// Print raw report summary.
fn print_raw_report(evidence: &serde_json::Value) {
    let report = &evidence["attestation_report"];
    if report.is_null() {
        return;
    }

    println!("── Raw Report ──");

    // Try to get a sense of the report size from fields
    // The attestation_report is 1184 bytes per AMD spec
    let report_str = serde_json::to_string(report).unwrap_or_default();
    println!(
        "{}...{} (JSON: {} bytes, binary: 1184 bytes)",
        &report_str[..40.min(report_str.len())],
        &report_str[report_str.len().saturating_sub(40)..],
        report_str.len()
    );
}

// === Helper functions ===

/// Print a field as-is.
fn print_field(obj: &serde_json::Value, key: &str, label: &str) {
    if let Some(val) = obj.get(key) {
        if let Some(n) = val.as_u64() {
            println!("{:14} {}", format!("{}:", label), n);
        } else if let Some(s) = val.as_str() {
            println!("{:14} {}", format!("{}:", label), s);
        }
    }
}

/// Print a field as hex (handles both string hex and numeric values).
fn print_hex_field(obj: &serde_json::Value, key: &str, label: &str) {
    if let Some(val) = obj.get(key) {
        let display = if let Some(s) = val.as_str() {
            // Already hex string - truncate if long
            if s.len() > 64 {
                format!("{}...{}", &s[..32], &s[s.len() - 32..])
            } else {
                s.to_string()
            }
        } else if let Some(n) = val.as_u64() {
            format!("{:#x}", n)
        } else if let Some(arr) = val.as_array() {
            // Array of bytes
            let bytes: Vec<u8> = arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect();
            let hex = hex::encode(&bytes);
            if hex.len() > 64 {
                format!("{}...{}", &hex[..32], &hex[hex.len() - 32..])
            } else {
                hex
            }
        } else {
            return;
        };
        println!("{:14} {}", format!("{}:", label), display);
    }
}

/// Extract bytes from a JSON field (hex string or array).
fn extract_bytes(obj: &serde_json::Value, key: &str) -> Option<Vec<u8>> {
    let val = obj.get(key)?;

    if let Some(hex_str) = val.as_str() {
        hex::decode(hex_str).ok()
    } else if let Some(arr) = val.as_array() {
        Some(arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect())
    } else {
        None
    }
}

/// Extract u64 from a JSON field.
fn extract_u64(obj: &serde_json::Value, key: &str) -> Option<u64> {
    obj.get(key)?.as_u64()
}
