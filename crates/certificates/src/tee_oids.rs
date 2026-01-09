// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! TEE-specific OID definitions for Intel SGX/TDX and AMD SEV-SNP certificates.
//!
//! Uses `x509-parser`'s `oid-registry` for OID lookup and extends it with TEE-specific OIDs.

use std::sync::LazyLock;
use x509_parser::der_parser::Oid;
use x509_parser::oid_registry::OidRegistry;

// OID arc prefixes for platform detection
pub const INTEL_SGX_OID_PREFIX: &str = "1.2.840.113741.1.13";
pub const AMD_SEV_OID_PREFIX: &str = "1.3.6.1.4.1.3704";

/// Global TEE OID registry (extends x509-parser's default registry)
pub static TEE_OID_REGISTRY: LazyLock<OidRegistry<'static>> = LazyLock::new(|| {
    use x509_parser::der_parser::oid;

    let mut reg = OidRegistry::default().with_all_crypto().with_x509();

    // Intel SGX/TDX OIDs
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1), ("SGX-Extensions", "SGX Extension Container"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .1), ("PPID", "Platform Provisioning ID"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .2), ("TCB", "TCB Component Versions"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .2 .17), ("PCESVN", "PCE Security Version"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .2 .18), ("CPUSVN", "CPU Security Version"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .3), ("PCE-ID", "PCE Identifier"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .4), ("FMSPC", "Family-Model-Stepping-Platform-CustomSKU"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .5), ("SGX-Type", "SGX Platform Type"));
    reg.insert(oid!(1.2.840 .113741 .1 .13 .1 .6), ("Platform-Instance-ID", "Platform Instance ID"));

    // AMD SEV-SNP OIDs
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .1), ("BlSpl", "Bootloader Security Patch Level"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .2), ("Product", "Product Name"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .1), ("LoaderSpl", "Loader Security Patch Level"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .2), ("TeeSpl", "TEE Security Patch Level"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .3), ("SnpSpl", "SNP Firmware Security Patch Level"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .4), ("Spl4", "Security Patch Level 4"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .5), ("Spl5", "Security Patch Level 5"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .6), ("Spl6", "Security Patch Level 6"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .7), ("Spl7", "Security Patch Level 7"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .8), ("UcodeSpl", "Microcode Security Patch Level"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .9), ("FmcSpl", "FMC Security Patch Level"));
    reg.insert(oid!(1.3.6 .1 .4 .1 .3704 .1 .4), ("HW-ID", "Hardware/Chip ID"));

    reg
});

/// Check if an OID string is a TEE-specific extension
pub fn is_tee_oid(oid_str: &str) -> bool {
    oid_str.starts_with(INTEL_SGX_OID_PREFIX) || oid_str.starts_with(AMD_SEV_OID_PREFIX)
}

/// Get TEE platform from OID string
pub fn tee_platform(oid_str: &str) -> Option<crate::quote_certs::TeePlatform> {
    use crate::quote_certs::TeePlatform;
    if oid_str.starts_with(INTEL_SGX_OID_PREFIX) {
        Some(TeePlatform::Intel)
    } else if oid_str.starts_with(AMD_SEV_OID_PREFIX) {
        Some(TeePlatform::Amd)
    } else {
        None
    }
}

/// Get short name for an OID, with fallback for unknown OIDs
pub fn oid_name(oid: &Oid) -> String {
    TEE_OID_REGISTRY.get(oid).map(|e| e.sn().to_string()).unwrap_or_else(|| format!("OID({})", oid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use x509_parser::der_parser::oid;

    #[test]
    fn test_registry_lookup() {
        let fmspc = oid!(1.2.840 .113741 .1 .13 .1 .4);
        assert_eq!(TEE_OID_REGISTRY.get(&fmspc).map(|e| e.sn()), Some("FMSPC"));

        let snp = oid!(1.3.6 .1 .4 .1 .3704 .1 .3 .3);
        assert_eq!(TEE_OID_REGISTRY.get(&snp).map(|e| e.sn()), Some("SnpSpl"));
    }

    #[test]
    fn test_platform_detection() {
        assert!(is_tee_oid("1.2.840.113741.1.13.1.4"));
        assert!(is_tee_oid("1.3.6.1.4.1.3704.1.4"));
        assert!(!is_tee_oid("2.5.29.14")); // Subject Key Identifier
    }
}
