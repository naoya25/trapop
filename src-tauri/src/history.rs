use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const HISTORY_FILE: &str = "history.jsonl";
const SOURCE_PREVIEW_CHARS: usize = 200;
const HISTORY_DISPLAY_LIMIT: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: String,
    pub timestamp_ms: i64,
    pub source_preview: String,
    pub translated_text: String,
    pub is_html: bool,
    pub engine: String,
}

impl HistoryRecord {
    pub fn new(source_text: &str, translated_text: String, is_html: bool, engine: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let timestamp_ms = now.as_millis() as i64;
        let id = format!("{timestamp_ms}-{:x}", now.subsec_nanos());
        let source_preview = text_preview(source_text, SOURCE_PREVIEW_CHARS);

        Self {
            id,
            timestamp_ms,
            source_preview,
            translated_text,
            is_html,
            engine: engine.to_string(),
        }
    }
}

fn text_preview(source: &str, max_chars: usize) -> String {
    let mut preview = String::new();
    let mut in_tag = false;
    for ch in source.chars() {
        if preview.chars().count() >= max_chars {
            break;
        }
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => preview.push(ch),
            _ => {}
        }
    }
    preview
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(HISTORY_FILE))
}

pub fn append(app: &AppHandle, record: &HistoryRecord) -> Result<(), String> {
    let path = history_path(app)?;
    let line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| e.to_string())
}

fn recent_from_lines(content: &str, limit: usize) -> Vec<HistoryRecord> {
    let mut records: Vec<HistoryRecord> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    records.reverse();
    records.truncate(limit);
    records
}

pub fn load_recent(app: &AppHandle) -> Result<Vec<HistoryRecord>, String> {
    let path = history_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    Ok(recent_from_lines(&content, HISTORY_DISPLAY_LIMIT))
}

pub fn find(app: &AppHandle, id: &str) -> Result<Option<HistoryRecord>, String> {
    Ok(load_recent(app)?.into_iter().find(|record| record.id == id))
}

pub fn clear(app: &AppHandle) -> Result<(), String> {
    let path = history_path(app)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_source_preview_and_strips_tags() {
        let long_text = "a".repeat(250);
        let record = HistoryRecord::new(&long_text, "translated".to_string(), false, "mock");
        assert_eq!(record.source_preview.chars().count(), SOURCE_PREVIEW_CHARS);

        let html_source = "<p>hello <strong>world</strong></p>";
        let record = HistoryRecord::new(html_source, "translated".to_string(), true, "mock");
        assert_eq!(record.source_preview, "hello world");
    }

    #[test]
    fn recent_from_lines_orders_newest_first_and_respects_limit() {
        let lines: Vec<String> = (0..3)
            .map(|i| {
                let record = HistoryRecord {
                    id: i.to_string(),
                    timestamp_ms: i,
                    source_preview: format!("source-{i}"),
                    translated_text: format!("translated-{i}"),
                    is_html: false,
                    engine: "mock".to_string(),
                };
                serde_json::to_string(&record).unwrap()
            })
            .collect();
        let content = lines.join("\n");

        let recent = recent_from_lines(&content, 2);

        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "2");
        assert_eq!(recent[1].id, "1");
    }

    #[test]
    fn recent_from_lines_skips_malformed_lines() {
        let content = "not json\n{\"broken\":true}\n";
        assert_eq!(recent_from_lines(content, 50).len(), 0);
    }
}
