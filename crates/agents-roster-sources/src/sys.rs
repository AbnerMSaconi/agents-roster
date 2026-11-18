// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 AbnerMSaconi

//! System clock, local timezone formatting, and process UID queries (zero `unsafe`).

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use agents_roster_core::{Clock, TimeFormatter, Translator};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};

/// Wall-clock time provider backed by `SystemTime`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Local timezone formatter backed by `chrono::Local`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChronoTimeFormatter;

impl TimeFormatter for ChronoTimeFormatter {
    fn format_time(&self, ms: u64) -> String {
        let Some(dt) = Local.timestamp_millis_opt(ms as i64).single() else {
            return String::new();
        };
        dt.format("%H:%M").to_string()
    }

    fn format_when(&self, ms: u64, now_ms: u64, tr: &dyn Translator) -> String {
        let (Some(target), Some(today)) = (
            Local.timestamp_millis_opt(ms as i64).single(),
            Local.timestamp_millis_opt(now_ms as i64).single(),
        ) else {
            return String::new();
        };
        if target.date_naive() == today.date_naive() {
            return target.format("%H:%M").to_string();
        }
        let day_key = match target.weekday() {
            Weekday::Sun => "Sun",
            Weekday::Mon => "Mon",
            Weekday::Tue => "Tue",
            Weekday::Wed => "Wed",
            Weekday::Thu => "Thu",
            Weekday::Fri => "Fri",
            Weekday::Sat => "Sat",
        };
        format!("{} {}", tr.translate(day_key), target.format("%H:%M"))
    }
}

/// Parses local time from a glog line (`E1115 13:41:18.274349 ...`) to Unix ms.
pub fn parse_glog_ms(line: &str, now_ms: u64) -> Option<u64> {
    let bytes = line.as_bytes();
    if bytes.len() < 21 || !matches!(bytes[0], b'I' | b'W' | b'E' | b'F') || bytes[5] != b' ' {
        return None;
    }
    let part = |idx: usize, len: usize| line.get(idx..idx + len)?.parse::<u32>().ok();
    let (month, day, hour, min, sec) = (
        part(1, 2)?,
        part(3, 2)?,
        part(6, 2)?,
        part(9, 2)?,
        part(12, 2)?,
    );
    let current_year = Local.timestamp_millis_opt(now_ms as i64).single()?.year();
    let time = NaiveTime::from_hms_opt(hour, min, sec)?;
    for year in [current_year, current_year - 1] {
        let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
            continue;
        };
        let naive = NaiveDateTime::new(date, time);
        if let Some(dt) = Local.from_local_datetime(&naive).earliest() {
            let ms = u64::try_from(dt.timestamp_millis()).ok()?;
            if ms <= now_ms + 86_400_000 {
                return Some(ms);
            }
        }
    }
    None
}

fn parse_proc_uid(index: usize) -> Option<u32> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(index + 1)?.parse().ok()
}

/// Real UID of the current process (from `/proc/self/status`).
pub fn current_uid() -> u32 {
    parse_proc_uid(0).unwrap_or(1000)
}

/// Returns `true` when effective UID is `0`.
pub fn is_effective_root() -> bool {
    parse_proc_uid(1) == Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agents_roster_core::IdentityTranslator;

    #[test]
    fn parses_glog_lines_relative_to_now() {
        let now = SystemClock.now_ms();
        assert!(
            parse_glog_ms("E1115 13:41:18.274349     945 errorreport.go:307] x", now).is_some()
        );
        assert!(parse_glog_ms("not a glog line", now).is_none());
    }

    #[test]
    fn formats_today_without_weekday_prefix() {
        let fmt = ChronoTimeFormatter;
        let now = SystemClock.now_ms();
        let text = fmt.format_when(now, now, &IdentityTranslator);
        assert_eq!(text.len(), 5);
        assert_eq!(&text[2..3], ":");
    }
}
