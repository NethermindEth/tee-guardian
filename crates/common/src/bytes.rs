// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Fixed-size byte array types with hex serialization.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

macro_rules! define_fixed_bytes {
    ($name:ident, $size:expr, $doc:expr) => {
        #[doc = $doc]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub [u8; $size]);

        impl $name {
            /// Create from a byte slice. Panics if slice length != $size.
            #[must_use]
            pub fn from_slice(slice: &[u8]) -> Self {
                let mut arr = [0u8; $size];
                arr.copy_from_slice(slice);
                Self(arr)
            }

            /// Returns the inner byte array.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; $size] {
                &self.0
            }

            /// Returns hex-encoded string.
            #[must_use]
            pub fn to_hex(&self) -> String {
                hex::encode(self.0)
            }

            /// Parse from a hex string. Returns error if invalid hex or wrong length.
            #[must_use]
            pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
                let bytes = hex::decode(s)?;
                if bytes.len() != $size {
                    return Err(hex::FromHexError::InvalidStringLength);
                }
                Ok(Self::from_slice(&bytes))
            }
        }

        impl From<[u8; $size]> for $name {
            fn from(arr: [u8; $size]) -> Self {
                Self(arr)
            }
        }

        impl From<$name> for [u8; $size] {
            fn from(b: $name) -> Self {
                b.0
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "0x{}", self.to_hex())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "0x{}", self.to_hex())
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_hex())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let s = String::deserialize(deserializer)?;
                let s = s.strip_prefix("0x").unwrap_or(&s);
                let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
                if bytes.len() != $size {
                    return Err(serde::de::Error::custom(format!("expected {} bytes, got {}", $size, bytes.len())));
                }
                Ok(Self::from_slice(&bytes))
            }
        }
    };
}

define_fixed_bytes!(Bytes48, 48, "48-byte array (used for TDX measurements like RTMR, mr_td, mr_seam).");
define_fixed_bytes!(Bytes32, 32, "32-byte array (used for SHA256 hashes like measurement_id, platform_id).");
define_fixed_bytes!(Bytes64, 64, "64-byte array (used for SHA512 hashes like mr_enclave).");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes48_roundtrip() {
        let original = Bytes48([42u8; 48]);
        let json = serde_json::to_string(&original).unwrap();
        let restored: Bytes48 = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_bytes32_roundtrip() {
        let original = Bytes32([42u8; 32]);
        let json = serde_json::to_string(&original).unwrap();
        let restored: Bytes32 = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_bytes48_hex_prefix() {
        let b = Bytes48([0u8; 48]);
        let json = format!("\"0x{}\"", "00".repeat(48));
        let restored: Bytes48 = serde_json::from_str(&json).unwrap();
        assert_eq!(b, restored);
    }

    #[test]
    fn test_bytes64_roundtrip() {
        let original = Bytes64([42u8; 64]);
        let json = serde_json::to_string(&original).unwrap();
        let restored: Bytes64 = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_bytes64_hex_prefix() {
        let b = Bytes64([0u8; 64]);
        let json = format!("\"0x{}\"", "00".repeat(64));
        let restored: Bytes64 = serde_json::from_str(&json).unwrap();
        assert_eq!(b, restored);
    }
}
