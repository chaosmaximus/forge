//! Shared timestamp utilities for Forge.
//!
//! All timestamps use ISO 8601 format matching SQLite `datetime('now')`:
//! `"2026-04-02 23:15:30"` (no trailing Z).
//!
//! Use `now_iso()` everywhere instead of local helpers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Produce ISO 8601 timestamp matching SQLite `datetime('now')` format.
/// Output: `"2026-04-02 23:15:30"` (no trailing Z — matches SQLite convention).
pub fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_to_iso(secs)
}

/// Produce ISO 8601 timestamp offset by `delta_secs` from now.
/// Positive = future, negative = past.
pub fn now_offset(delta_secs: i64) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let adjusted = if delta_secs >= 0 {
        secs.saturating_add(delta_secs as u64)
    } else {
        secs.saturating_sub(delta_secs.unsigned_abs())
    };
    epoch_to_iso(adjusted)
}

/// Produce Unix epoch seconds as a string (for event timestamps).
pub fn timestamp_now() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

/// Convert epoch seconds to ISO 8601 string.
pub fn epoch_to_iso(secs: u64) -> String {
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let mut year = 1970u64;
    let mut remaining_days = days_since_epoch;
    loop {
        let is_leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days_in_year = if is_leap { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let is_leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_days: [u64; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for &days in &month_days {
        if remaining_days < days {
            break;
        }
        remaining_days -= days;
        month += 1;
    }
    let day = remaining_days + 1;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_iso_format() {
        let ts = now_iso();
        // Format: "YYYY-MM-DD HH:MM:SS"
        assert_eq!(ts.len(), 19);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], " ");
        assert_eq!(&ts[13..14], ":");
    }

    #[test]
    fn test_now_offset_future() {
        let now = now_iso();
        let future = now_offset(3600); // 1 hour ahead
        assert!(future > now);
    }

    #[test]
    fn test_now_offset_past() {
        let now = now_iso();
        let past = now_offset(-3600); // 1 hour ago
        assert!(past < now);
    }

    #[test]
    fn test_timestamp_now() {
        let ts = timestamp_now();
        let secs: u64 = ts.parse().unwrap();
        assert!(secs > 1_700_000_000); // After 2023
    }

    #[test]
    fn test_epoch_to_iso_known_value() {
        // 2026-01-01 00:00:00 UTC = 1767225600
        let ts = epoch_to_iso(1767225600);
        assert_eq!(ts, "2026-01-01 00:00:00");
    }
}
