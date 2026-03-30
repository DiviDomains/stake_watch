use chrono::NaiveDateTime;

/// Byte-reverse a hex string (used for ZMQ block hashes).
///
/// Splits into 2-char pairs, reverses the order, and rejoins.
/// Example: `"abcdef12"` -> `"12efcdab"`
pub fn reverse_hex(hex: &str) -> String {
    let chars: Vec<char> = hex.chars().collect();
    let mut pairs: Vec<&[char]> = chars.chunks(2).collect();
    pairs.reverse();
    pairs.iter().flat_map(|pair| pair.iter()).collect()
}

/// Format a satoshi amount as a human-readable DIVI string with 8 decimal places.
///
/// Example: `123_456_780_000` -> `"1234.56780000"`
pub fn satoshi_to_divi(satoshis: i64) -> String {
    let abs = satoshis.unsigned_abs();
    let whole = abs / 100_000_000;
    let frac = abs % 100_000_000;
    let sign = if satoshis < 0 { "-" } else { "" };
    format!("{sign}{whole}.{frac:08}")
}

/// Truncate a blockchain address to `first6...last6` form for display.
///
/// If the address is 12 characters or shorter it is returned as-is.
pub fn truncate_address(address: &str) -> String {
    if address.len() <= 12 {
        return address.to_string();
    }
    let first = &address[..6];
    let last = &address[address.len() - 6..];
    format!("{first}...{last}")
}

/// Produce a human-friendly relative time string from a `NaiveDateTime` to now.
///
/// Examples: "just now", "2 minutes ago", "3 hours ago", "5 days ago".
pub fn time_ago(timestamp: &NaiveDateTime) -> String {
    let now = chrono::Utc::now().naive_utc();
    let delta = now.signed_duration_since(*timestamp);

    let secs = delta.num_seconds();
    if secs < 0 {
        return "in the future".to_string();
    }
    if secs < 60 {
        return "just now".to_string();
    }

    let minutes = delta.num_minutes();
    if minutes < 60 {
        return if minutes == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{minutes} minutes ago")
        };
    }

    let hours = delta.num_hours();
    if hours < 24 {
        return if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hours} hours ago")
        };
    }

    let days = delta.num_days();
    if days == 1 {
        "1 day ago".to_string()
    } else {
        format!("{days} days ago")
    }
}

/// Format a duration in seconds into a compact human-readable string.
///
/// Examples: `135` -> `"2m 15s"`, `90061` -> `"1d 1h"`.
pub fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        return "0s".to_string();
    }

    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    let mut parts: Vec<String> = Vec::new();

    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 && days == 0 {
        // skip minutes when showing days
        parts.push(format!("{minutes}m"));
    }
    if secs > 0 && days == 0 && hours == 0 {
        // skip seconds when showing hours+
        parts.push(format!("{secs}s"));
    }

    if parts.is_empty() {
        // Edge case: e.g. exactly 86400 seconds but days > 0 already handled
        return "0s".to_string();
    }

    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse_hex_basic() {
        assert_eq!(reverse_hex("abcdef12"), "12efcdab");
    }

    #[test]
    fn test_reverse_hex_single_byte() {
        assert_eq!(reverse_hex("ab"), "ab");
    }

    #[test]
    fn test_reverse_hex_empty() {
        assert_eq!(reverse_hex(""), "");
    }

    #[test]
    fn test_reverse_hex_block_hash() {
        // Typical 64-char block hash
        let input = "0000000000000003fa5efb1ae2f6c5e7a8d1b3c4e5f67890abcdef1234567890";
        let reversed = reverse_hex(input);
        assert_eq!(reversed.len(), 64);
        // Reversing twice yields the original
        assert_eq!(reverse_hex(&reversed), input);
    }

    #[test]
    fn test_satoshi_to_divi_zero() {
        assert_eq!(satoshi_to_divi(0), "0.00000000");
    }

    #[test]
    fn test_satoshi_to_divi_positive() {
        assert_eq!(satoshi_to_divi(100_000_000), "1.00000000");
        assert_eq!(satoshi_to_divi(123_456_780_000), "1234.56780000");
        assert_eq!(satoshi_to_divi(1), "0.00000001");
    }

    #[test]
    fn test_satoshi_to_divi_negative() {
        assert_eq!(satoshi_to_divi(-500_000_000), "-5.00000000");
    }

    #[test]
    fn test_satoshi_to_divi_large() {
        assert_eq!(
            satoshi_to_divi(100_000_000_000_000),
            "1000000.00000000"
        );
    }

    #[test]
    fn test_truncate_address_short() {
        assert_eq!(truncate_address("D12345"), "D12345");
    }

    #[test]
    fn test_truncate_address_normal() {
        let addr = "D8nQRyfgS5xL7dZDC39i9s41iiCAEeq7Zk";
        // first 6 = "D8nQRy", last 6 = "Eeq7Zk"
        assert_eq!(truncate_address(addr), "D8nQRy...Eeq7Zk");
    }

    #[test]
    fn test_truncate_address_exact() {
        let addr = "D8nQRyfgS5xL7dZDC39i9s41iiCAEeq7Zk";
        let t = truncate_address(addr);
        // first 6 chars
        assert!(t.starts_with(&addr[..6]));
        // last 6 chars
        assert!(t.ends_with(&addr[addr.len() - 6..]));
        assert!(t.contains("..."));
    }

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration(0), "0s");
    }

    #[test]
    fn test_format_duration_seconds_only() {
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn test_format_duration_minutes_seconds() {
        assert_eq!(format_duration(135), "2m 15s");
    }

    #[test]
    fn test_format_duration_hours_minutes() {
        assert_eq!(format_duration(3600 * 2 + 60 * 15), "2h 15m");
    }

    #[test]
    fn test_format_duration_days_hours() {
        assert_eq!(format_duration(86400 * 3 + 3600 * 4), "3d 4h");
    }
}
