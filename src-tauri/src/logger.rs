use crate::platform::windows::WatcherEvent;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

const DATE_FORMAT: &[time::format_description::FormatItem<'static>] =
    format_description!("[year]-[month]-[day]");

pub struct EventLogger {
    log_dir: PathBuf,
}

impl EventLogger {
    pub fn new(log_dir: PathBuf) -> Self {
        Self { log_dir }
    }

    pub fn default_log_dir() -> Result<PathBuf, String> {
        let base_dir = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "LOCALAPPDATA is not available".to_string())?;
        Ok(base_dir.join("smart-shift").join("logs"))
    }

    pub fn current_log_path(&self) -> Result<PathBuf, String> {
        let now = local_now();
        let file_name = now
            .format(DATE_FORMAT)
            .map_err(|e| format!("failed to format log date: {e}"))?;
        Ok(self.log_dir.join(format!("{file_name}.log")))
    }

    pub fn append_event(&self, event: &WatcherEvent) -> Result<PathBuf, String> {
        let path = self.current_log_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create log directory {}: {e}", parent.display()))?;
        }

        let timestamp = local_now()
            .format(&Rfc3339)
            .map_err(|e| format!("failed to format log timestamp: {e}"))?;
        let payload = serde_json::json!({
            "timestamp": timestamp,
            "event": watcher_event_value(event),
        });

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("failed to open log file {}: {e}", path.display()))?;
        writeln!(file, "{payload}")
            .map_err(|e| format!("failed to write log file {}: {e}", path.display()))?;

        Ok(path)
    }

    pub fn read_recent_lines(&self, limit: usize) -> Result<Vec<String>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let path = self.current_log_path()?;
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|e| format!("failed to open log file {}: {e}", path.display()))?;
        let mut lines = BufReader::new(file)
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("failed to read log file {}: {e}", path.display()))?;
        if lines.len() > limit {
            lines = lines.split_off(lines.len() - limit);
        }
        Ok(lines)
    }
}

fn watcher_event_value(event: &WatcherEvent) -> Value {
    serde_json::json!({
        "line_text": event.line_text,
        "source": event.source,
        "current_mode": event.current_mode,
        "target_mode": event.target_mode,
        "switched": event.switched,
        "preserved": event.preserved,
        "reason": event.reason,
        "error": event.error,
    })
}

fn local_now() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    match UtcOffset::current_local_offset() {
        Ok(offset) => now.to_offset(offset),
        Err(_) => now,
    }
}

#[cfg(test)]
mod tests {
    use super::EventLogger;
    use crate::platform::windows::WatcherEvent;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("smart-shift-logger-test-{suffix}"))
    }

    fn test_event(line_text: &str) -> WatcherEvent {
        WatcherEvent {
            line_text: line_text.to_string(),
            source: "win32_edit".to_string(),
            current_mode: Some("english".to_string()),
            target_mode: Some("chinese".to_string()),
            switched: true,
            preserved: false,
            reason: "neighbor_cjk".to_string(),
            error: None,
        }
    }

    fn remove_dir_if_exists(path: &Path) {
        if path.exists() {
            fs::remove_dir_all(path).expect("failed to clean temporary log directory");
        }
    }

    #[test]
    fn appends_json_log_lines() {
        let temp_dir = unique_temp_dir();
        remove_dir_if_exists(&temp_dir);

        let logger = EventLogger::new(temp_dir.clone());
        let path = logger
            .append_event(&test_event("hello"))
            .expect("append event");
        let content = fs::read_to_string(&path).expect("read log file");

        assert!(content.contains("\"line_text\":\"hello\""));
        assert!(content.contains("\"reason\":\"neighbor_cjk\""));

        remove_dir_if_exists(&temp_dir);
    }

    #[test]
    fn reads_only_requested_tail_lines() {
        let temp_dir = unique_temp_dir();
        remove_dir_if_exists(&temp_dir);

        let logger = EventLogger::new(temp_dir.clone());
        logger
            .append_event(&test_event("first"))
            .expect("append first line");
        logger
            .append_event(&test_event("second"))
            .expect("append second line");
        logger
            .append_event(&test_event("third"))
            .expect("append third line");

        let lines = logger.read_recent_lines(2).expect("read recent lines");

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"line_text\":\"second\""));
        assert!(lines[1].contains("\"line_text\":\"third\""));

        remove_dir_if_exists(&temp_dir);
    }
}
