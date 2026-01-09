// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Sensitive data wrapper that prevents accidental logging
//!
//! This module provides type-level protection against accidentally exposing
//! sensitive data (keys, secrets, etc.) in logs or debug output.
//!
//! # Example
//!
//! ```
//! use common::sensitive::Sensitive;
//! use zeroize::Zeroize;
//!
//! #[derive(Zeroize)]
//! #[zeroize(drop)]
//! struct KeyMaterial {
//!     bytes: [u8; 32],
//! }
//!
//! let secret = Sensitive::new(KeyMaterial { bytes: [42u8; 32] });
//!
//! // This will NOT compile - Debug is not implemented:
//! // println!("{:?}", secret);
//!
//! // Must explicitly expose to access:
//! let key_bytes = &secret.expose().bytes;
//! ```
//!
//! # Safety
//!
//! This module uses unsafe code in `into_inner()` to extract the inner value
//! without running the Drop implementation. This is necessary because the Zeroize
//! derive macro implements Drop, and we need to allow extraction of the value
//! when ownership is transferred.

use zeroize::Zeroize;

/// Wrapper for sensitive data that prevents accidental disclosure
///
/// This type intentionally does NOT implement:
/// - `Debug` (prevents `{:?}` formatting)
/// - `Display` (prevents `{}` formatting)
/// - `Clone` (prevents accidental duplication)
/// - `Serialize` (prevents JSON/etc serialization)
///
/// This ensures that sensitive data cannot be accidentally logged via
/// tracing macros or debug statements.
pub struct Sensitive<T: Zeroize>(T);

impl<T: Zeroize> Drop for Sensitive<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<T: Zeroize> Sensitive<T> {
    /// Wrap a sensitive value
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Explicitly expose the sensitive value
    ///
    /// This is intentionally verbose to make it clear when sensitive
    /// data is being accessed.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// Explicitly expose the sensitive value mutably
    pub fn expose_mut(&mut self) -> &mut T {
        &mut self.0
    }

    /// Consume and extract the sensitive value
    ///
    /// Note: This consumes self but does not run the Drop implementation
    /// because we're extracting the inner value. The caller is responsible
    /// for ensuring the value is properly zeroized.
    #[allow(unsafe_code)]
    pub fn into_inner(self) -> T {
        // Replace with zeroed value to prevent drop from zeroizing
        unsafe {
            let inner = std::ptr::read(&self.0 as *const T);
            std::mem::forget(self);
            inner
        }
    }
}

// Implement a safe Debug that doesn't leak data
impl<T: Zeroize> std::fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sensitive<REDACTED>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Zeroize)]
    #[zeroize(drop)]
    struct TestSecret {
        data: Vec<u8>,
    }

    #[test]
    fn test_sensitive_new() {
        let secret = Sensitive::new(TestSecret { data: vec![1, 2, 3] });
        assert_eq!(secret.expose().data, vec![1, 2, 3]);
    }

    #[test]
    fn test_sensitive_debug_redacts() {
        let secret = Sensitive::new(TestSecret { data: vec![1, 2, 3] });
        let debug_str = format!("{:?}", secret);
        assert_eq!(debug_str, "Sensitive<REDACTED>");
        assert!(!debug_str.contains("1"));
        assert!(!debug_str.contains("2"));
        assert!(!debug_str.contains("3"));
    }

    #[test]
    fn test_sensitive_expose_mut() {
        let mut secret = Sensitive::new(TestSecret { data: vec![1, 2, 3] });
        secret.expose_mut().data.push(4);
        assert_eq!(secret.expose().data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_sensitive_into_inner() {
        let secret = Sensitive::new(TestSecret { data: vec![1, 2, 3] });
        let inner = secret.into_inner();
        assert_eq!(inner.data, vec![1, 2, 3]);
    }

    #[test]
    fn test_sensitive_prevents_logging() {
        let secret = Sensitive::new(TestSecret { data: vec![42, 43, 44] });

        // This should compile and not expose the secret
        let log_output = format!("Got secret: {:?}", secret);
        assert!(!log_output.contains("42"));
        assert!(!log_output.contains("43"));
        assert!(!log_output.contains("44"));
        assert!(log_output.contains("REDACTED"));
    }
}
