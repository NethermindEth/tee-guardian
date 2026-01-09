// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

// Copyright (c) 2024 Nethermind
//
// SPDX-License-Identifier: Apache-2.0

//! TDX Information Tool
//!
//! Displays TDX attestation information by:
//! 1. Generating a quote via the attester crate
//! 2. Parsing it via the verifier crate to get JSON claims
//! 3. Parsing certificates via the certificates crate

use attester::{tdx::TdxAttester, Attester};
use base64::Engine;
use certificates::parse_quote_signature;
use std::path::Path;
use std::process;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Check TDX availability
    let tsm_available = Path::new("/sys/kernel/config/tsm/report").exists();
    let tdx_device = Path::new("/dev/tdx_guest").exists();

    if !tsm_available && !tdx_device {
        return Err(format!(
            "TDX not available\n  /dev/tdx_guest: {}\n  /sys/kernel/config/tsm/report: {}",
            if tdx_device { "ok" } else { "missing" },
            if tsm_available { "ok" } else { "missing" }
        )
        .into());
    }

    // Generate attestation using attester crate
    let rt = tokio::runtime::Runtime::new()?;

    let (quote_bytes, claims) = rt.block_on(async {
        // 1. Get raw evidence from attester
        let attester = TdxAttester::default();
        let evidence =
            attester.get_evidence(vec![0u8; 64]).await.map_err(|e| format!("Failed to get evidence: {e}"))?;

        // Extract quote bytes
        let quote_b64 = evidence["quote"].as_str().ok_or("Missing quote field in evidence")?;
        let quote_bytes =
            base64::prelude::BASE64_STANDARD.decode(quote_b64).map_err(|e| format!("Failed to decode quote: {e}"))?;

        // 2. Parse via verifier to get claims
        use kbs_types::Tee;
        use verifier::{InitDataHash, ReportData};

        let tdx_verifier =
            verifier::to_verifier(&Tee::Tdx, None).await.map_err(|e| format!("Failed to create verifier: {e}"))?;

        let claims_result = tdx_verifier
            .evaluate(evidence, &ReportData::NotProvided, &InitDataHash::NotProvided)
            .await
            .map_err(|e| format!("Verification failed: {e}"))?;

        let claims = if !claims_result.is_empty() {
            claims_result.into_iter().next().unwrap().0
        } else {
            serde_json::Value::Null
        };

        Ok::<_, String>((quote_bytes, claims))
    })?;

    // Print the report
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                          TDX ATTESTATION REPORT                              ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();

    // Print parsed claims
    print_claims(&claims);

    // Print certificate chain
    println!();
    println!("┌──────────────────────────────────────────────────────────────────────────────┐");
    println!("│                            CERTIFICATE CHAIN                                 │");
    println!("└──────────────────────────────────────────────────────────────────────────────┘");

    let sig_offset = calculate_sig_offset(&quote_bytes);
    match parse_quote_signature(&quote_bytes, sig_offset) {
        Ok(sig) => {
            println!("signature:      {}... ({} bytes)", hex::encode(&sig.signature[..16]), sig.signature.len());
            println!(
                "attest_key:     {}... ({} bytes)",
                hex::encode(&sig.attest_pub_key[..16]),
                sig.attest_pub_key.len()
            );

            if let Some(ref dt) = sig.cert_data_type {
                println!("cert_type:      {} ({})", dt, *dt as u16);
                println!("cert_size:      {} bytes", sig.cert_data_size);
            }

            if let Some(ref chain) = sig.cert_chain {
                println!();
                for (i, cert) in chain.certs.iter().enumerate() {
                    let label = match i {
                        0 => "PCK (Leaf)",
                        _ if i == chain.certs.len() - 1 => "Root CA",
                        _ => "Intermediate",
                    };
                    println!("── {} {} ──", i + 1, label);
                    print!("{}", cert);
                    println!();
                }
            }
        }
        Err(e) => println!("Certificate parse error: {}", e),
    }

    // Raw quote summary
    println!("── Raw Quote ──");
    let hex_quote = hex::encode(&quote_bytes);
    println!(
        "{}...{} ({} bytes)",
        &hex_quote[..40.min(hex_quote.len())],
        &hex_quote[hex_quote.len().saturating_sub(40)..],
        quote_bytes.len()
    );

    Ok(())
}

/// Print the parsed claims JSON in a readable format.
fn print_claims(claims: &serde_json::Value) {
    // TCB info (from DCAP verification)
    if let Some(tcb) = claims.get("tcb_status").and_then(|v| v.as_str()) {
        println!("tcb_status:     {}", tcb);
    }
    if let Some(date) = claims.get("tcb_date").and_then(|v| v.as_str()) {
        println!("tcb_date:       {}", date);
    }
    if let Some(ids) = claims.get("advisory_ids").and_then(|v| v.as_array()) {
        if !ids.is_empty() {
            let ids_str: Vec<_> = ids.iter().filter_map(|v| v.as_str()).collect();
            println!("advisory_ids:   {}", ids_str.join(", "));
        }
    }

    // Quote header
    if let Some(header) = claims.get("quote").and_then(|q| q.get("header")) {
        println!();
        println!("── Quote Header ──");
        print_hex_field(header, "version", "version");
        print_hex_field(header, "tee_type", "tee_type");
        print_hex_field(header, "att_key_type", "att_key_type");
        print_hex_field(header, "vendor_id", "vendor_id");
    }

    // Quote body (TD Report)
    if let Some(body) = claims.get("quote").and_then(|q| q.get("body")) {
        println!();
        println!("── TD Report ──");
        print_hex_field(body, "tcb_svn", "tcb_svn");
        print_hex_field(body, "mr_td", "mr_td");
        print_hex_field(body, "mr_seam", "mr_seam");
        print_hex_field(body, "td_attributes", "td_attributes");
        print_hex_field(body, "xfam", "xfam");

        println!();
        println!("── RTMRs ──");
        print_hex_field(body, "rtmr_0", "RTMR[0]");
        print_hex_field(body, "rtmr_1", "RTMR[1]");
        print_hex_field(body, "rtmr_2", "RTMR[2]");
        print_hex_field(body, "rtmr_3", "RTMR[3]");

        println!();
        println!("── Report Data ──");
        print_hex_field(body, "report_data", "report_data");
    }

    // TD Attributes breakdown
    if let Some(attrs) = claims.get("td_attributes") {
        println!();
        println!("── TD Attributes ──");
        if let Some(debug) = attrs.get("debug").and_then(|v| v.as_bool()) {
            println!("DEBUG:          {}", debug);
        }
        if let Some(sept) = attrs.get("septve_disable").and_then(|v| v.as_bool()) {
            println!("SEPT_VE_DIS:    {}", sept);
        }
    }
}

fn print_hex_field(obj: &serde_json::Value, key: &str, label: &str) {
    if let Some(val) = obj.get(key).and_then(|v| v.as_str()) {
        println!("{:14} {}", format!("{}:", label), val);
    }
}

/// Calculate signature data offset based on quote version.
///
/// - Quote v4: Header (48) + Body (584) = 632
/// - Quote v5: Header (48) + Type (2) + Size (4) + Body (584 or 648)
fn calculate_sig_offset(quote: &[u8]) -> usize {
    if quote.len() < 48 {
        return 0;
    }

    let version = u16::from_le_bytes([quote[0], quote[1]]);

    match version {
        4 => 48 + 584, // 632
        5 => {
            // v5 has type (2 bytes) after header
            if quote.len() < 54 {
                return 0;
            }
            let quote_type = u16::from_le_bytes([quote[48], quote[49]]);
            let body_size = match quote_type {
                2 => 584, // TDX 1.0
                3 => 648, // TDX 1.5
                _ => 584, // Default
            };
            48 + 2 + 4 + body_size // header + type + size + body
        }
        _ => 632, // Default to v4
    }
}
