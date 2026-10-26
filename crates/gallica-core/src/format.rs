//! Human readable formatting shared by the front ends.

use std::time::Duration;

/// `1.2 MB`, `830 KB`, `12 B`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `1.2 MB/s`.
pub fn rate(bytes_per_second: f64) -> String {
    format!("{}/s", bytes(bytes_per_second.max(0.0) as u64))
}

/// `2d 03h`, `1h 02m`, `4m 05s`, `12s`.
pub fn duration(d: Duration) -> String {
    let s = d.as_secs();
    let (days, hours, minutes, seconds) = (s / 86_400, (s / 3600) % 24, (s / 60) % 60, s % 60);
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// `12 345` with thin grouping, for counts.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(12), "12 B");
        assert_eq!(bytes(830_000), "830 KB");
        assert_eq!(bytes(1_234_567), "1.2 MB");
        assert_eq!(duration(Duration::from_secs(12)), "12s");
        assert_eq!(duration(Duration::from_secs(245)), "4m 05s");
        assert_eq!(duration(Duration::from_secs(3720)), "1h 02m");
        assert_eq!(duration(Duration::from_secs(183_600)), "2d 03h");
        assert_eq!(count(12_345), "12\u{202f}345");
        assert_eq!(count(999), "999");
    }
}
