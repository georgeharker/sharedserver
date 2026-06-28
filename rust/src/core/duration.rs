use anyhow::{bail, Result};
use std::time::Duration;

/// Parse duration string like "5m", "1h", "2h30m", "90s"
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        bail!("Empty duration string");
    }

    let mut total_secs = 0u64;
    let mut current_num = String::new();

    // Add `value * unit_secs` to the running total, rejecting overflow instead of
    // panicking (debug) or silently wrapping to a tiny duration (release).
    let add = |value: u64, unit_secs: u64, total: &mut u64| -> Result<()> {
        let contribution = value
            .checked_mul(unit_secs)
            .ok_or_else(|| anyhow::anyhow!("Duration too large: {}", s))?;
        *total = total
            .checked_add(contribution)
            .ok_or_else(|| anyhow::anyhow!("Duration too large: {}", s))?;
        Ok(())
    };

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current_num.push(ch);
        } else if ch == 'h' || ch == 'H' {
            add(current_num.parse()?, 3600, &mut total_secs)?;
            current_num.clear();
        } else if ch == 'm' || ch == 'M' {
            add(current_num.parse()?, 60, &mut total_secs)?;
            current_num.clear();
        } else if ch == 's' || ch == 'S' {
            add(current_num.parse()?, 1, &mut total_secs)?;
            current_num.clear();
        } else if ch.is_whitespace() {
            continue;
        } else {
            bail!("Invalid character in duration: {}", ch);
        }
    }

    if !current_num.is_empty() {
        bail!("Duration must end with unit (h, m, or s): {}", s);
    }

    if total_secs == 0 {
        bail!("Duration must be greater than zero");
    }

    Ok(Duration::from_secs(total_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("5m").unwrap().as_secs(), 300);
        assert_eq!(parse_duration("1h").unwrap().as_secs(), 3600);
        assert_eq!(parse_duration("2h30m").unwrap().as_secs(), 9000);
        assert_eq!(parse_duration("90s").unwrap().as_secs(), 90);
        assert_eq!(parse_duration("1h30m45s").unwrap().as_secs(), 5445);

        assert!(parse_duration("").is_err());
        assert!(parse_duration("5").is_err());
        assert!(parse_duration("5x").is_err());
        assert!(parse_duration("0m").is_err());
    }

    #[test]
    fn test_parse_duration_overflow_is_error() {
        // Parses as u64 but *3600 overflows -> error, not a panic (debug) or a
        // silently-wrapped tiny duration (release).
        assert!(parse_duration("9223372036854775807h").is_err());
        // Overflow while summing across units.
        assert!(parse_duration("9999999999999999999s9999999999999999999s").is_err());
    }
}
