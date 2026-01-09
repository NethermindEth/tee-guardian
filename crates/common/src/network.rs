// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Network utilities.

use std::net::Ipv4Addr;
use thiserror::Error;
use tracing::info;

/// Path to DHCP option 224 file containing hex-encoded IP.
const DHCP_OPTION_224_PATH: &str = "/var/lib/dhcp/option-224";

/// Error type for network operations.
#[derive(Debug, Error)]
pub enum NetworkError {
    /// Failed to read DHCP option file.
    #[error("Failed to read DHCP option 224: {0}")]
    DhcpReadError(#[from] std::io::Error),

    /// Invalid hex IP format.
    #[error("Invalid hex IP format: expected 8 characters, got {0}")]
    InvalidHexLength(usize),

    /// Failed to parse hex IP.
    #[error("Failed to parse hex IP: {0}")]
    HexParseError(String),
}

/// Get node IP from DHCP option 224.
///
/// Reads hex-encoded IPv4 address from `/var/lib/dhcp/option-224`.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read
/// - The hex string is not 8 characters
/// - The hex cannot be parsed as IPv4 octets
pub fn get_node_ip_from_dhcp() -> Result<Ipv4Addr, NetworkError> {
    let hex_ip = std::fs::read_to_string(DHCP_OPTION_224_PATH)?.trim().to_string();

    let ip = parse_hex_ip(&hex_ip)?;
    info!("Public IP from DHCP option 224: {} -> {}", hex_ip, ip);
    Ok(ip)
}

/// Parse hex-encoded IPv4 address (e.g., "c0a87a14" -> 192.168.122.20).
fn parse_hex_ip(hex_ip: &str) -> Result<Ipv4Addr, NetworkError> {
    if hex_ip.len() != 8 {
        return Err(NetworkError::InvalidHexLength(hex_ip.len()));
    }

    let octets: Result<Vec<u8>, _> = (0..4).map(|i| u8::from_str_radix(&hex_ip[i * 2..i * 2 + 2], 16)).collect();

    let octets = octets.map_err(|e| NetworkError::HexParseError(e.to_string()))?;
    Ok(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_ip() {
        // 192.168.122.20
        assert_eq!(parse_hex_ip("c0a87a14").unwrap(), Ipv4Addr::new(192, 168, 122, 20));
        // 10.0.0.1
        assert_eq!(parse_hex_ip("0a000001").unwrap(), Ipv4Addr::new(10, 0, 0, 1));
        // 127.0.0.1
        assert_eq!(parse_hex_ip("7f000001").unwrap(), Ipv4Addr::new(127, 0, 0, 1));
        // 255.255.255.255
        assert_eq!(parse_hex_ip("ffffffff").unwrap(), Ipv4Addr::new(255, 255, 255, 255));
    }

    #[test]
    fn test_parse_hex_ip_invalid_length() {
        assert!(matches!(parse_hex_ip("c0a8"), Err(NetworkError::InvalidHexLength(4))));
        assert!(matches!(parse_hex_ip("c0a87a14ff"), Err(NetworkError::InvalidHexLength(10))));
        assert!(matches!(parse_hex_ip(""), Err(NetworkError::InvalidHexLength(0))));
    }

    #[test]
    fn test_parse_hex_ip_invalid_hex() {
        assert!(matches!(parse_hex_ip("zzzzzzzz"), Err(NetworkError::HexParseError(_))));
        assert!(matches!(parse_hex_ip("c0a87a1g"), Err(NetworkError::HexParseError(_))));
    }
}
