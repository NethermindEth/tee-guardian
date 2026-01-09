// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nethermind

//! Time utilities.

use std::time::{SystemTime, UNIX_EPOCH};

/// Get current Unix timestamp in seconds.
#[inline]
#[must_use]
pub fn unix_timestamp_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Get current Unix timestamp in microseconds.
#[inline]
#[must_use]
pub fn unix_timestamp_micros() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as i64)
}
