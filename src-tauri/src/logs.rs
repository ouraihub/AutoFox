//! In-app backend log buffer with daily persistent files.
//!
//! The UI reads a bounded tail instead of consuming stdout. Each line is
//! written to `<exe>\data\logs\YYYY-MM-DD.log` in UTC and mirrored to the
//! stable `autofox.log` path for compatibility.
//!
//! # Why URLs are redacted
//!
//! Media URLs carry signed query strings: `sig`, `expire`, `token`. A log the
//! user pastes into a bug report would otherwise hand over a working
//! credential, so **every URL written here must go through [`redact_url`]**.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    sync::{LazyLock, Mutex},
};

use crate::system::paths;
use serde::Serialize;

const TAIL_BYTES: u64 = 256 * 1024;

/// A backend log level ordered from most severe to most verbose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// An operation failed.
    Error,
    /// An operation degraded but continued.
    Warn,
    /// Normal lifecycle progress.
    Info,
    /// Detail useful when diagnosing a failure.
    Debug,
    /// Per-chunk detail.
    Trace,
}

impl LogLevel {
    /// Parses a persisted setting, falling back to INFO.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "ERROR" => Self::Error,
            "WARN" | "WARNING" => Self::Warn,
            "DEBUG" => Self::Debug,
            "TRACE" => Self::Trace,
            _ => Self::Info,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

/// Strips the query and fragment from a URL, keeping origin and path.
///
/// Parsing is deliberately textual: this runs on strings a backend already
/// accepted, and a redactor that can reject its input would be a redactor
/// that sometimes logs the raw URL instead.
#[must_use]
pub fn redact_url(url: &str) -> String {
    let cut = url.find(['?', '#']);
    match cut {
        Some(index) => format!("{}?…", &url[..index]),
        None => url.to_owned(),
    }
}

/// One log line exposed to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Monotonic sequence number within this process.
    pub seq: u64,
    /// Wall-clock time of day in UTC, `HH:MM:SS.mmm`.
    pub time: String,
    /// Log level.
    pub level: String,
    /// Source module.
    pub target: String,
    /// Human-readable message.
    pub message: String,
}

/// Bounded process-local log state.
#[derive(Default)]
pub struct LogState {
    entries: Mutex<Vec<LogEntry>>,
    next_seq: Mutex<u64>,
}

impl LogState {
    const MAX_ENTRIES: usize = 1000;

    /// Appends a line and discards the oldest line over the bound.
    pub fn append(&self, level: LogLevel, target: &str, message: impl Into<String>) -> LogEntry {
        self.append_with_date(level, target, message).0
    }

    fn append_with_date(
        &self,
        level: LogLevel,
        target: &str,
        message: impl Into<String>,
    ) -> (LogEntry, String) {
        // Sequence numbers stay unique after old lines are evicted.
        let seq = self.next_sequence();
        let (date, time) = now_utc();
        let entry = LogEntry {
            seq,
            time,
            level: level.as_str().to_owned(),
            target: target.to_owned(),
            message: sanitize_message(message.into()),
        };
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry.clone());
            if entries.len() > Self::MAX_ENTRIES {
                let remove = entries.len() - Self::MAX_ENTRIES;
                entries.drain(..remove);
            }
        }
        (entry, date)
    }

    fn next_sequence(&self) -> u64 {
        match self.next_seq.lock() {
            Ok(mut next) => {
                *next += 1;
                *next
            }
            Err(_) => 0,
        }
    }

    fn restore_from_file() -> Self {
        let state = Self::default();
        for entry in read_startup_tail() {
            let seq = state.next_sequence();
            if let Ok(mut entries) = state.entries.lock() {
                entries.push(LogEntry { seq, ..entry });
            }
        }
        state
    }

    /// Returns newest entries first, filtered by minimum severity.
    #[must_use]
    pub fn tail(&self, limit: usize, minimum: LogLevel) -> Vec<LogEntry> {
        let Ok(entries) = self.entries.lock() else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|entry| LogLevel::parse(&entry.level) <= minimum)
            .rev()
            .take(limit.clamp(1, 1000))
            .cloned()
            .collect()
    }

    /// Clears all buffered lines.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}

static LOGS: LazyLock<LogState> = LazyLock::new(LogState::restore_from_file);

/// Returns the process-wide log buffer.
///
/// A global rather than managed Tauri state: the download and mux paths log
/// from inside spawned tasks that never receive an `AppHandle`.
pub fn logs() -> &'static LogState {
    &LOGS
}

fn record(level: LogLevel, target: &str, message: impl Into<String>) {
    let (entry, date) = logs().append_with_date(level, target, message);
    // Daily file is canonical. The stable legacy filename remains a mirror.
    let _ = append_file(&entry, &date);
}

/// Records an `ERROR` line.
pub fn error(target: &str, message: impl Into<String>) {
    record(LogLevel::Error, target, message);
}

/// Records a `WARN` line.
pub fn warn(target: &str, message: impl Into<String>) {
    record(LogLevel::Warn, target, message);
}

/// Records an `INFO` line.
pub fn info(target: &str, message: impl Into<String>) {
    record(LogLevel::Info, target, message);
}

/// Records a `DEBUG` line.
pub fn debug(target: &str, message: impl Into<String>) {
    record(LogLevel::Debug, target, message);
}

/// Reads the in-app log tail, filtered by the configured level.
#[tauri::command]
pub fn read_logs(
    settings: tauri::State<'_, crate::settings::SettingsState>,
    limit: usize,
) -> Vec<LogEntry> {
    let limit = if limit == 0 { 300 } else { limit };
    let level = settings
        .0
        .lock()
        .ok()
        .map_or(LogLevel::Info, |value| LogLevel::parse(&value.log_level));
    logs().tail(limit, level)
}

/// Clears the in-app buffer and all persistent log feeds.
#[tauri::command]
pub fn clear_logs() {
    logs().clear();
    clear_persistent_files();
}

fn clear_persistent_files() {
    let logs_dir = paths::data_dir().join("logs");
    let _ = fs::remove_file(paths::log_file());
    let Ok(entries) = fs::read_dir(logs_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_daily_log_name)
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn is_daily_log_name(name: &str) -> bool {
    name.len() == 14
        && name.as_bytes()[4] == b'-'
        && name.as_bytes()[7] == b'-'
        && name.ends_with(".log")
        && name[..4].chars().all(|ch| ch.is_ascii_digit())
        && name[5..7].chars().all(|ch| ch.is_ascii_digit())
        && name[8..10].chars().all(|ch| ch.is_ascii_digit())
}

/// Returns the current UTC date and time in `YYYY-MM-DD` / `HH:MM:SS.mmm` form.
fn now_utc() -> (String, String) {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let total_seconds = millis / 1000;
    let days = (total_seconds / 86_400) as i64;
    let seconds = total_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hours = seconds / 3600;
    let minutes = (seconds / 60) % 60;
    let seconds = seconds % 60;
    let fraction = millis % 1000;
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!("{hours:02}:{minutes:02}:{seconds:02}.{fraction:03}"),
    )
}

// Howard Hinnant's proleptic-Gregorian conversion from Unix epoch days.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = (if days >= 0 { days } else { days - 146_096 }) / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let year = year + if month_prime < 10 { 0 } else { 1 };
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    (year, month, day)
}

/// Serializes a log entry into one parseable, newline-safe file line.
fn format_file_line(entry: &LogEntry) -> String {
    format!(
        "{} {} {}: {}\n",
        entry.time, entry.level, entry.target, entry.message
    )
}

fn append_file(entry: &LogEntry, date: &str) -> std::io::Result<()> {
    append_to_paths(entry, &paths::daily_log_file(date), &paths::log_file())
}

fn append_to_paths(
    entry: &LogEntry,
    daily_path: &std::path::Path,
    compatibility_path: &std::path::Path,
) -> std::io::Result<()> {
    let line = format_file_line(entry);
    let mut first_error = None;
    for path in [daily_path, compatibility_path] {
        let result = (|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?
                .write_all(line.as_bytes())
        })();
        if let Err(error) = result
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn read_startup_tail() -> Vec<LogEntry> {
    let (date, _) = now_utc();
    read_startup_tail_from_paths(&paths::daily_log_file(&date), &paths::log_file())
}

fn read_startup_tail_from_paths(
    daily_path: &std::path::Path,
    compatibility_path: &std::path::Path,
) -> Vec<LogEntry> {
    let daily = read_file_tail(daily_path);
    if daily.is_empty() {
        read_file_tail(compatibility_path)
    } else {
        daily
    }
}

fn read_file_tail(path: &std::path::Path) -> Vec<LogEntry> {
    let Ok(mut file) = OpenOptions::new().read(true).open(path) else {
        return Vec::new();
    };
    let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
        return Vec::new();
    };
    let offset = length.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return Vec::new();
    }
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return Vec::new();
    }
    let mut lines = text.lines();
    if offset > 0 {
        let _ = lines.next();
    }
    lines
        .filter_map(parse_file_line)
        .rev()
        .take(LogState::MAX_ENTRIES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn parse_file_line(line: &str) -> Option<LogEntry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (time, remainder) = line.split_once(' ')?;
    let (level, remainder) = remainder.split_once(' ')?;
    let (target, message) = remainder.split_once(": ")?;
    Some(LogEntry {
        seq: 0,
        time: time.to_owned(),
        level: level.to_owned(),
        target: target.to_owned(),
        message: message.to_owned(),
    })
}

fn sanitize_message(message: String) -> String {
    message
        .split_whitespace()
        .map(|word| {
            if word.starts_with("http://") || word.starts_with("https://") {
                redact_url(word)
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_signed_query_string() {
        let redacted = redact_url("https://cdn.example.com/v/1.mp4?sig=abc123&expire=99");
        assert_eq!(redacted, "https://cdn.example.com/v/1.mp4?…");
        assert!(!redacted.contains("abc123"));
    }

    #[test]
    fn keeps_url_without_query_intact() {
        assert_eq!(
            redact_url("https://example.com/a/b.mp4"),
            "https://example.com/a/b.mp4"
        );
    }

    #[test]
    fn redacts_fragment_too() {
        assert_eq!(
            redact_url("https://example.com/p#token=x"),
            "https://example.com/p?…"
        );
    }

    #[test]
    fn tail_filters_lines_below_configured_level() {
        let state = LogState::default();
        state.append(LogLevel::Debug, "t", "verbose");
        state.append(LogLevel::Error, "t", "broke");
        let at_info = state.tail(10, LogLevel::Info);
        assert_eq!(at_info.len(), 1);
        assert_eq!(at_info[0].message, "broke");
        assert_eq!(state.tail(10, LogLevel::Debug).len(), 2);
    }

    #[test]
    fn tail_returns_newest_first() {
        let state = LogState::default();
        state.append(LogLevel::Info, "t", "first");
        state.append(LogLevel::Info, "t", "second");
        let tail = state.tail(10, LogLevel::Info);
        assert_eq!(tail[0].message, "second");
        assert_eq!(tail[1].message, "first");
    }

    #[test]
    fn buffer_is_bounded_and_keeps_newest() {
        let state = LogState::default();
        for index in 0..(LogState::MAX_ENTRIES + 5) {
            state.append(LogLevel::Info, "t", index.to_string());
        }
        let tail = state.tail(1000, LogLevel::Trace);
        assert_eq!(tail.len(), LogState::MAX_ENTRIES);
        assert_eq!(tail[0].message, (LogState::MAX_ENTRIES + 4).to_string());
    }

    #[test]
    fn sequence_numbers_stay_unique_after_eviction() {
        let state = LogState::default();
        for _ in 0..(LogState::MAX_ENTRIES + 3) {
            state.append(LogLevel::Info, "t", "x");
        }
        let tail = state.tail(1000, LogLevel::Trace);
        let mut seqs: Vec<u64> = tail.iter().map(|entry| entry.seq).collect();
        seqs.sort_unstable();
        seqs.dedup();
        assert_eq!(seqs.len(), tail.len());
    }

    #[test]
    fn clear_empties_the_buffer() {
        let state = LogState::default();
        state.append(LogLevel::Info, "t", "x");
        state.clear();
        assert!(state.tail(10, LogLevel::Trace).is_empty());
    }

    #[test]
    fn writes_daily_file_and_legacy_mirror() {
        let root = std::env::temp_dir().join(format!(
            "autofox-log-test-{}-{}",
            std::process::id(),
            now_utc().1.replace([':', '.'], "")
        ));
        let daily = root.join("2026-09-26.log");
        let legacy = root.join("autofox.log");
        let entry = LogEntry {
            seq: 1,
            time: "12:34:56.789".to_owned(),
            level: "INFO".to_owned(),
            target: "test".to_owned(),
            message: "persisted".to_owned(),
        };
        append_to_paths(&entry, &daily, &legacy).expect("write both feeds");
        assert_eq!(read_file_tail(&daily)[0].message, "persisted");
        assert_eq!(read_file_tail(&legacy)[0].message, "persisted");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_prefers_daily_file_without_duplicating_legacy_mirror() {
        let root = std::env::temp_dir().join(format!(
            "autofox-log-prefer-{}-{}",
            std::process::id(),
            now_utc().1.replace([':', '.'], "")
        ));
        let daily = root.join("2026-09-26.log");
        let legacy = root.join("autofox.log");
        let daily_entry = LogEntry {
            seq: 1,
            time: "12:34:56.789".to_owned(),
            level: "INFO".to_owned(),
            target: "daily".to_owned(),
            message: "one".to_owned(),
        };
        let legacy_entry = LogEntry {
            target: "legacy".to_owned(),
            ..daily_entry.clone()
        };
        append_to_paths(&daily_entry, &daily, &root.join("unused.log")).expect("daily write");
        append_to_paths(&legacy_entry, &legacy, &root.join("unused-2.log")).expect("legacy write");
        let restored = read_startup_tail_from_paths(&daily, &legacy);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].target, "daily");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_falls_back_to_legacy_when_daily_file_is_missing() {
        let root = std::env::temp_dir().join(format!(
            "autofox-log-fallback-{}-{}",
            std::process::id(),
            now_utc().1.replace([':', '.'], "")
        ));
        let daily = root.join("missing.log");
        let legacy = root.join("autofox.log");
        let entry = LogEntry {
            seq: 1,
            time: "12:34:56.789".to_owned(),
            level: "INFO".to_owned(),
            target: "legacy".to_owned(),
            message: "restored".to_owned(),
        };
        append_to_paths(&entry, &root.join("unused.log"), &legacy).expect("legacy write");
        let restored = read_startup_tail_from_paths(&daily, &legacy);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].message, "restored");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn utc_date_and_time_have_expected_width() {
        let (date, time) = now_utc();
        assert_eq!(date.len(), 10);
        assert_eq!(date.as_bytes()[4], b'-');
        assert_eq!(date.as_bytes()[7], b'-');
        assert_eq!(time.len(), 12);
        assert_eq!(time.as_bytes()[2], b':');
        assert_eq!(time.as_bytes()[8], b'.');
    }
    #[test]
    fn sanitizes_urls_and_newlines_at_sink_boundary() {
        let message = sanitize_message("url https://cdn.example/v.mp4?sig=secret\nnext".to_owned());
        assert_eq!(message, "url https://cdn.example/v.mp4?… next");
        assert!(!message.contains("secret"));
    }

    #[test]
    fn file_lines_round_trip() {
        let entry = LogEntry {
            seq: 1,
            time: "12:34:56.789".to_owned(),
            level: "INFO".to_owned(),
            target: "test".to_owned(),
            message: "hello".to_owned(),
        };
        assert_eq!(
            parse_file_line(&format_file_line(&entry)).map(|v| v.message),
            Some("hello".to_owned())
        );
    }
}
