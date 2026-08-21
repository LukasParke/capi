//! Small shared helpers.

use std::time::Duration;

/// Parse a query `wait` value: plain seconds ("5") or a duration ("5s", "500ms").
pub fn parse_wait(s: &str) -> Result<Duration, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(Duration::ZERO);
    }
    if let Ok(n) = t.parse::<u64>() {
        return Ok(Duration::from_secs(n));
    }
    // Accept Go-style durations: 500ms, 2s, 1m30s. Every letter run must
    // follow a number; bare letters ("abc") are an error.
    let mut total = Duration::ZERO;
    let mut num = String::new();
    let mut unit = String::new();
    for c in t.chars() {
        if c.is_ascii_digit() || c == '.' {
            if !unit.is_empty() {
                return Err(format!("invalid duration {t:?}"));
            }
            num.push(c);
        } else {
            unit.push(c);
        }
    }
    if num.is_empty() {
        return Err(format!("invalid duration {t:?}"));
    }
    total += apply_unit(&num, &unit)?;
    Ok(total)
}

fn apply_unit(num: &str, unit: &str) -> Result<Duration, String> {
    let v: f64 = num
        .parse()
        .map_err(|_| format!("invalid duration {num}{unit}"))?;
    match unit {
        "ms" => Ok(Duration::from_millis(v as u64)),
        "s" | "" => Ok(Duration::from_secs_f64(v)),
        "m" => Ok(Duration::from_secs_f64(v * 60.0)),
        other => Err(format!("unknown duration unit {other:?}")),
    }
}

/// Title-case helper replacing deprecated strings.Title usage.
pub fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for c in s.chars() {
        if cap {
            out.extend(c.to_uppercase());
            cap = false;
        } else {
            out.push(c);
        }
        if c.is_whitespace() || c == '-' || c == '_' {
            cap = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_parsing() {
        assert_eq!(parse_wait("5").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_wait("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_wait("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_wait("").unwrap(), Duration::ZERO);
        assert!(parse_wait("abc").is_err());
        assert!(parse_wait("-3").is_err());
    }

    #[test]
    fn title() {
        assert_eq!(title_case("standby"), "Standby");
        assert_eq!(title_case("on"), "On");
    }
}
