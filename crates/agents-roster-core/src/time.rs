// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! Pure duration and timestamp parsing/formatting helpers.

/// Formats milliseconds as `12 s`, `4 min`, or `2 h 05`.
pub fn format_duration(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min", s / 60),
        _ => format!("{} h {:02}", s / 3600, (s % 3600) / 60),
    }
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant algorithm).
fn civil_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let mp = (m + 9) % 12;
    let day_of_year = (153 * mp + 2) / 5 + d - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn parse_tz_offset_secs(rest: &str) -> i64 {
    let after_frac = match rest.strip_prefix('.') {
        Some(r) => r.trim_start_matches(|c: char| c.is_ascii_digit()),
        None => rest,
    };
    let sign = match after_frac.as_bytes().first() {
        Some(b'+') => 1i64,
        Some(b'-') => -1i64,
        _ => return 0,
    };
    let tz = &after_frac[1..];
    let (hh, mm) = tz.split_once(':').unwrap_or((tz, "0"));
    let h = hh.get(..2).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    let m = mm.get(..2).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    sign * (h * 3600 + m * 60)
}

/// Parses ISO-8601 timestamps (`Z` or `[+-]HH:MM`, optional fractional seconds) to Unix ms.
pub fn parse_iso_ms(iso: &str) -> Option<u64> {
    let bytes = iso.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let part = |idx: usize, len: usize| iso.get(idx..idx + len)?.parse::<i64>().ok();
    let (year, month, day) = (part(0, 4)?, part(5, 2)?, part(8, 2)?);
    let (hour, min, sec) = (part(11, 2)?, part(14, 2)?, part(17, 2)?);
    let rest = &iso[19..];
    let frac = parse_frac_ms(rest);
    let offset_secs = parse_tz_offset_secs(rest);
    let epoch_secs =
        civil_days(year, month, day) * 86_400 + hour * 3600 + min * 60 + sec - offset_secs;
    u64::try_from(epoch_secs * 1000 + frac).ok()
}

fn parse_frac_ms(rest: &str) -> i64 {
    let Some(r) = rest.strip_prefix('.') else {
        return 0;
    };
    let digits: String = r.chars().take_while(char::is_ascii_digit).collect();
    let three: String = digits.chars().chain("000".chars()).take(3).collect();
    three.parse().unwrap_or(0)
}

/// Parses Go duration strings (`4h51m32s`, `12m`, `30s`) into milliseconds.
pub fn parse_go_duration_ms(s: &str) -> Option<u64> {
    let mut total = 0u64;
    let mut num = String::new();
    let mut found = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let v: u64 = num.parse().ok()?;
            num.clear();
            let factor = match c {
                'h' => 3_600_000,
                'm' => 60_000,
                's' => 1000,
                _ => return None,
            };
            total += v * factor;
            found = true;
        }
    }
    (found && num.is_empty()).then_some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_timestamps_with_offsets() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2026-11-15T17:38:09Z"),
            Some(1_794_764_289_000)
        );
        assert_eq!(
            parse_iso_ms("2026-11-15T13:38:09-04:00"),
            Some(1_794_764_289_000)
        );
        assert_eq!(
            parse_iso_ms("2026-11-15T17:38:09.5Z"),
            Some(1_794_764_289_500)
        );
        assert_eq!(parse_iso_ms("garbage"), None);
    }

    #[test]
    fn parses_go_durations() {
        assert_eq!(
            parse_go_duration_ms("4h51m32s"),
            Some((4 * 3600 + 51 * 60 + 32) * 1000)
        );
        assert_eq!(parse_go_duration_ms("30s"), Some(30_000));
        assert_eq!(parse_go_duration_ms("4h51"), None);
        assert_eq!(parse_go_duration_ms(""), None);
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(12_000), "12 s");
        assert_eq!(format_duration(240_000), "4 min");
        assert_eq!(format_duration(7_500_000), "2 h 05");
    }
}
