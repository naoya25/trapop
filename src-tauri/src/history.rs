use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const HISTORY_FILE: &str = "history.jsonl";
const SOURCE_PREVIEW_CHARS: usize = 200;
const HISTORY_DISPLAY_LIMIT: usize = 50;
const HISTORY_MAX_RECORDS: usize = 500;
// 1レコードの訳文上限。無制限だと 500件×長文でファイルが肥大化し、
// append ごとの rotate 全読みが重くなる
const HISTORY_TRANSLATED_MAX_CHARS: usize = 50_000;
// rotate の全読みはファイルがこのサイズを超えるまで走らせない(通常は metadata 参照のみ)
const ROTATE_TRIGGER_BYTES: u64 = 4 * 1024 * 1024;
// rotate 実行時はこのバイト数以下まで古い行を落とす。トリガーの半分にすることで
// 「トリガー超えっぱなし → 毎 append 全読み」に張り付かない
const ROTATE_TARGET_BYTES: usize = 2 * 1024 * 1024;

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
        let source_preview = text_preview(source_text, SOURCE_PREVIEW_CHARS, is_html);
        let translated_text = if translated_text.chars().count() > HISTORY_TRANSLATED_MAX_CHARS {
            translated_text
                .chars()
                .take(HISTORY_TRANSLATED_MAX_CHARS)
                .collect()
        } else {
            translated_text
        };

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

// タグ剥がしは HTML ソースのみ。plain text に '<' が含まれるケース
// (コード片の `if (a < b)` 等)を巻き込まないため is_html で分岐する。
fn text_preview(source: &str, max_chars: usize, strip_tags: bool) -> String {
    if !strip_tags {
        return source.chars().take(max_chars).collect();
    }

    let mut preview = String::new();
    let mut count = 0;
    let mut in_tag = false;
    for ch in source.chars() {
        if count >= max_chars {
            break;
        }
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                preview.push(ch);
                count += 1;
            }
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

// append + rotate は read-all → write-all を含み非原子。複数 popup の翻訳が
// 同時完了しても履歴が消えないよう、プロセス内 lock で直列化する。
static APPEND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// バイトトリガー未満でも件数上限が効くよう、N 回に1回は全読み rotate を強制する
// (超過は最大でもこの間隔ぶんに収まる)
static APPEND_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const FORCE_ROTATE_EVERY_APPENDS: u32 = 50;

pub fn append(app: &AppHandle, record: &HistoryRecord) -> Result<(), String> {
    let _guard = APPEND_LOCK.lock().map_err(|e| e.to_string())?;
    let path = history_path(app)?;
    let line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| e.to_string())?;
    drop(file);

    rotate_if_needed(&path)
}

// 翻訳対象は業務テキストなので、無期限に平文が溜まり続けないよう件数上限で切り詰める。
// 全読みは重いので、ファイルサイズが閾値を超えるまでは metadata 参照だけで済ませる。
fn rotate_if_needed(path: &PathBuf) -> Result<(), String> {
    // == 0 なので各プロセスの初回 append でも1回走る。カウンタはプロセス内なので、
    // 毎日再起動する使い方でも再起動をまたいだ件数上限が効く
    let count = APPEND_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let force_by_count = count.is_multiple_of(FORCE_ROTATE_EVERY_APPENDS);

    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if !force_by_count && size < ROTATE_TRIGGER_BYTES {
        return Ok(());
    }

    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();

    // 件数上限とバイト目標の両方で新しい側から残す
    let mut keep: Vec<&str> = lines
        .iter()
        .rev()
        .take(HISTORY_MAX_RECORDS)
        .copied()
        .collect();
    let mut total: usize = keep.iter().map(|l| l.len() + 1).sum();
    while total > ROTATE_TARGET_BYTES {
        match keep.pop() {
            Some(dropped) => total -= dropped.len() + 1,
            None => break,
        }
    }
    keep.reverse();

    let mut rewritten = keep.join("\n");
    rewritten.push('\n');
    fs::write(path, rewritten).map_err(|e| e.to_string())
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
