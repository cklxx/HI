use std::path::{Component, Path, PathBuf};
use std::{fmt::Write, fs};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Datelike, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::fs::{self as async_fs, OpenOptions};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{agent::AgentOutcome, llm::LlmLogEntry, tasks::Intent};

mod structured_text;
pub use structured_text::{
    LoadedStructuredTextPreview, StructuredContent, StructuredSection, StructuredTextHistoryEntry,
    StructuredTextHistoryFilters, delete_structured_text_preview, list_structured_text_history,
    load_structured_text_history_entry, load_structured_text_preview,
    restore_structured_text_preview_from_history, save_structured_text_preview,
};

const REQUIRED_DIRS: &[&str] = &[
    "intent/inbox",
    "intent/queue",
    "intent/queue/failed",
    "intent/inbox/deferred",
    "intent/history",
    "journals",
    "sp",
    "logs/llm",
    "mock",
    "mock/text_structure_history",
];

pub fn ensure_data_layout(data_dir: &Path) -> anyhow::Result<()> {
    for dir in REQUIRED_DIRS {
        let path = data_dir.join(dir);
        fs::create_dir_all(&path).with_context(|| format!("creating dir {:?}", path))?;
    }
    Ok(())
}

pub fn load_yaml<T: DeserializeOwned>(path: PathBuf) -> anyhow::Result<T> {
    let content = fs::read_to_string(&path).with_context(|| format!("reading yaml {:?}", path))?;
    let parsed =
        serde_yaml::from_str(&content).with_context(|| format!("parsing yaml {:?}", path))?;
    Ok(parsed)
}

pub async fn write_markdown(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        async_fs::create_dir_all(parent).await?;
    }
    let mut file = async_fs::File::create(path).await?;
    file.write_all(content.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

pub fn list_markdown_files(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext == "md")
                .unwrap_or(false)
        })
        .map(|entry| entry.into_path())
        .collect()
}

pub fn list_markdown_tree(data_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut files: Vec<String> = list_markdown_files(data_dir)
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix(data_dir)
                .ok()
                .map(|relative| relative.to_string_lossy().to_string())
        })
        .collect();
    files.sort();
    Ok(files)
}

pub fn sanitize_data_relative_path(path: &str) -> anyhow::Result<PathBuf> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err(anyhow!("path must be relative"));
    }

    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(anyhow!("parent directory segments are not allowed"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(anyhow!("invalid path component"));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(anyhow!("path must not be empty"));
    }

    Ok(normalized)
}

pub async fn read_markdown_file(data_dir: &Path, relative_path: &Path) -> anyhow::Result<String> {
    let canonical_data = fs::canonicalize(data_dir)?;
    let absolute_path = data_dir.join(relative_path);
    if absolute_path.extension().and_then(|ext| ext.to_str()) != Some("md") {
        return Err(anyhow!("only markdown files may be read"));
    }

    let canonical_file = fs::canonicalize(&absolute_path)
        .with_context(|| format!("reading markdown at {:?}", relative_path))?;
    if !canonical_file.starts_with(&canonical_data) {
        return Err(anyhow!("path escapes data directory"));
    }

    let content = async_fs::read_to_string(canonical_file).await?;
    Ok(content)
}

#[derive(Debug, Clone)]
pub struct LlmLogQuery {
    pub model: Option<String>,
    pub run_id: Option<Uuid>,
    pub phase: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: usize,
}

impl Default for LlmLogQuery {
    fn default() -> Self {
        Self {
            model: None,
            run_id: None,
            phase: None,
            since: None,
            limit: 100,
        }
    }
}

pub async fn append_llm_logs(data_dir: &Path, entries: &[LlmLogEntry]) -> anyhow::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    for entry in entries {
        let date = entry.timestamp.date_naive();
        let log_dir =
            data_dir
                .join("logs/llm")
                .join(format!("{:04}/{:02}", date.year(), date.month()));
        async_fs::create_dir_all(&log_dir).await?;
        let log_path = log_dir.join(format!("{:02}.jsonl", date.day()));
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;
        let serialized = serde_json::to_string(entry)?;
        file.write_all(serialized.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
    }

    Ok(())
}

pub async fn read_llm_logs(
    data_dir: &Path,
    mut query: LlmLogQuery,
) -> anyhow::Result<Vec<LlmLogEntry>> {
    if query.limit == 0 {
        query.limit = 100;
    }

    let log_root = data_dir.join("logs/llm");
    if !log_root.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<PathBuf> = WalkDir::new(&log_root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    files.reverse();

    let mut results = Vec::new();
    for file in files {
        let content = async_fs::read_to_string(&file).await?;
        let mut lines: Vec<&str> = content.lines().collect();
        lines.reverse();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let entry: LlmLogEntry = serde_json::from_str(line)?;

            if let Some(ref model) = query.model {
                let matches_model = entry
                    .model
                    .as_ref()
                    .map(|value| value.eq_ignore_ascii_case(model))
                    .unwrap_or(false);
                if !matches_model {
                    continue;
                }
            }

            if let Some(ref phase) = query.phase
                && !entry.phase.eq_ignore_ascii_case(phase)
            {
                continue;
            }

            if query
                .run_id
                .as_ref()
                .is_some_and(|run_id| &entry.run_id != run_id)
            {
                continue;
            }

            if query
                .since
                .as_ref()
                .is_some_and(|since| &entry.timestamp < since)
            {
                continue;
            }

            results.push(entry);
            if results.len() >= query.limit {
                return Ok(results);
            }
        }
    }

    Ok(results)
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct IntentFrontMatter {
    #[serde(default)]
    id: Option<Uuid>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    telos_alignment: Option<f32>,
    #[serde(default)]
    created_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug)]
pub struct IntentRecord {
    pub path: PathBuf,
    pub intent: Intent,
}

#[derive(Debug)]
pub struct PersistedIntent {
    pub id: Uuid,
    pub path: PathBuf,
}

pub fn scan_inbox(data_dir: &Path) -> anyhow::Result<Vec<IntentRecord>> {
    let inbox_dir = data_dir.join("intent/inbox");
    scan_intent_dir(&inbox_dir)
}

pub fn scan_queue(data_dir: &Path) -> anyhow::Result<Vec<IntentRecord>> {
    let queue_dir = data_dir.join("intent/queue");
    scan_intent_dir(&queue_dir)
}

fn scan_intent_dir(dir: &Path) -> anyhow::Result<Vec<IntentRecord>> {
    let mut records = Vec::new();

    if !dir.exists() {
        return Ok(records);
    }

    for entry in fs::read_dir(dir).with_context(|| format!("reading intent dir at {:?}", dir))? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        let content = fs::read_to_string(&path)
            .with_context(|| format!("reading intent front matter at {:?}", path))?;
        let front_matter = parse_intent_front_matter(&content)?;
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("intent");

        let intent = Intent {
            id: front_matter.id.unwrap_or_else(Uuid::new_v4),
            source: front_matter.source.unwrap_or_else(|| "unknown".to_string()),
            summary: front_matter.summary.unwrap_or_else(|| stem.to_string()),
            telos_alignment: front_matter.telos_alignment.unwrap_or_default(),
            created_at: front_matter.created_at.unwrap_or_else(Utc::now),
            storage_path: Some(path.clone()),
        };

        records.push(IntentRecord { path, intent });
    }
    records.sort_by_key(|record| record.intent.created_at);
    Ok(records)
}

fn parse_intent_front_matter(content: &str) -> anyhow::Result<IntentFrontMatter> {
    let trimmed = content.trim_start();
    let yaml_block = if let Some(rest) = trimmed.strip_prefix("---") {
        let rest = rest.trim_start_matches(['\n', '\r']);
        if let Some(end) = rest.find("\n---") {
            &rest[..end]
        } else {
            rest
        }
    } else {
        trimmed.split("\n\n").next().unwrap_or_default()
    };

    if yaml_block.trim().is_empty() {
        return Ok(IntentFrontMatter::default());
    }

    let parsed = serde_yaml::from_str(yaml_block).with_context(|| "parsing intent front matter")?;
    Ok(parsed)
}

pub async fn persist_intent(
    data_dir: &Path,
    source: &str,
    summary: &str,
    telos_alignment: f32,
    body: &str,
) -> anyhow::Result<PersistedIntent> {
    let inbox_dir = data_dir.join("intent/inbox");
    async_fs::create_dir_all(&inbox_dir).await?;

    let created_at = Utc::now();
    let id = Uuid::new_v4();
    let file_name = format!("{}-{}.md", created_at.format("%Y%m%dT%H%M%S"), id);
    let path = inbox_dir.join(&file_name);

    let front_matter = IntentFrontMatter {
        id: Some(id),
        source: Some(source.to_string()),
        summary: Some(summary.to_string()),
        telos_alignment: Some(telos_alignment),
        created_at: Some(created_at),
    };

    let mut yaml = serde_yaml::to_string(&front_matter)?;
    if let Some(stripped) = yaml.strip_prefix("---\n") {
        yaml = stripped.to_string();
    }
    if let Some(stripped) = yaml.strip_suffix("...\n") {
        yaml = stripped.to_string();
    }
    let yaml = yaml.trim_end();

    let mut content = String::from("---\n");
    if !yaml.is_empty() {
        content.push_str(yaml);
        content.push('\n');
    }
    content.push_str("---\n\n");
    if !body.is_empty() {
        content.push_str(body);
        if !body.ends_with('\n') {
            content.push('\n');
        }
    }

    write_markdown(&path, &content).await?;

    Ok(PersistedIntent { id, path })
}

pub fn promote_to_queue(path: &Path, data_dir: &Path) -> anyhow::Result<PathBuf> {
    let queue_dir = data_dir.join("intent/queue");
    fs::create_dir_all(&queue_dir)
        .with_context(|| format!("ensuring queue dir {:?}", queue_dir))?;

    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("intent path missing file name: {:?}", path))?;
    let destination = queue_dir.join(file_name);
    fs::rename(path, &destination)
        .with_context(|| format!("moving intent to queue: {:?}", path))?;
    Ok(destination)
}

pub fn defer_intent(path: &Path, data_dir: &Path) -> anyhow::Result<PathBuf> {
    let deferred_dir = data_dir.join("intent/inbox/deferred");
    fs::create_dir_all(&deferred_dir)
        .with_context(|| format!("ensuring deferred dir {:?}", deferred_dir))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("intent path missing file name: {:?}", path))?;
    let destination = deferred_dir.join(file_name);
    fs::rename(path, &destination)
        .with_context(|| format!("moving intent to deferred: {:?}", path))?;
    Ok(destination)
}

pub fn quarantine_failed_intent(path: &Path, data_dir: &Path) -> anyhow::Result<PathBuf> {
    let failed_dir = data_dir.join("intent/queue/failed");
    fs::create_dir_all(&failed_dir)
        .with_context(|| format!("ensuring failed dir {:?}", failed_dir))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("intent path missing file name: {:?}", path))?;
    let destination = failed_dir.join(file_name);
    fs::rename(path, &destination)
        .with_context(|| format!("moving intent to failed queue: {:?}", path))?;
    Ok(destination)
}

pub async fn append_journal_entry(
    data_dir: &Path,
    intent: &Intent,
    outcome: &AgentOutcome,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let journal_dir = data_dir
        .join("journals")
        .join(format!("{:04}", now.year()))
        .join(format!("{:02}", now.month()));
    async_fs::create_dir_all(&journal_dir).await?;

    let journal_path = journal_dir.join(format!("{:02}.md", now.day()));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal_path)
        .await?;

    let mut trace = String::new();
    for (idx, step) in outcome.steps.iter().enumerate() {
        let _ = writeln!(
            &mut trace,
            "{}. Thought: {}\n   Action: {}\n   Observation: {}",
            idx + 1,
            step.thought,
            step.action,
            step.observation
        );
    }

    if trace.is_empty() {
        trace.push_str("(no ReAct steps recorded)\n");
    }

    let entry = format!(
        "## {} — {}\n\nIntent processed: {}\nFinal answer: {}\n\n### ReAct trace\n{}\n",
        now.format("%H:%M:%S"),
        intent.summary,
        intent.summary,
        outcome.final_answer,
        trace.trim_end(),
    );

    file.write_all(entry.as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

pub async fn archive_intent(intent: &Intent, data_dir: &Path) -> anyhow::Result<()> {
    let Some(path) = intent.storage_path.as_ref() else {
        return Ok(());
    };

    if !path.exists() {
        return Ok(());
    }

    let history_dir = data_dir.join("intent/history");
    async_fs::create_dir_all(&history_dir).await?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("intent path missing file name: {:?}", path))?;
    let destination = history_dir.join(file_name);
    async_fs::rename(path, destination).await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct SpIndex {
    #[serde(default)]
    pub top_used: Vec<String>,
    #[serde(default)]
    pub most_recent: Vec<String>,
}

pub async fn load_sp_index(data_dir: &Path) -> anyhow::Result<SpIndex> {
    let path = data_dir.join("sp/index.json");
    let content = async_fs::read_to_string(&path).await?;
    let persisted: PersistedSpIndex =
        serde_json::from_str(&content).with_context(|| "parsing sp/index.json")?;

    let top_used = persisted
        .top_used
        .iter()
        .map(|entry| format!("{} ({})", entry.summary, entry.count))
        .collect();
    let most_recent = persisted
        .most_recent
        .iter()
        .map(|entry| entry.summary.clone())
        .collect();

    Ok(SpIndex {
        top_used,
        most_recent,
    })
}

pub async fn update_sp_index(
    data_dir: &Path,
    intent: &Intent,
    outcome: &AgentOutcome,
) -> anyhow::Result<()> {
    let index_path = data_dir.join("sp/index.json");
    if let Some(parent) = index_path.parent() {
        async_fs::create_dir_all(parent).await?;
    }

    let mut index = if async_fs::try_exists(&index_path).await? {
        let content = async_fs::read_to_string(&index_path).await?;
        serde_json::from_str::<PersistedSpIndex>(&content)?
    } else {
        PersistedSpIndex::default()
    };

    let now = Utc::now();
    let summary = format!("{} ⇒ {}", intent.summary, outcome.final_answer);
    upsert_top_used(&mut index.top_used, &summary, now);
    upsert_most_recent(&mut index.most_recent, &summary, now);

    let serialized = serde_json::to_string_pretty(&index)?;
    async_fs::write(&index_path, serialized).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedSpIndex {
    #[serde(default)]
    top_used: Vec<SpEntry>,
    #[serde(default)]
    most_recent: Vec<SpEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpEntry {
    summary: String,
    count: u32,
    last_seen: DateTime<Utc>,
}

fn upsert_top_used(entries: &mut Vec<SpEntry>, summary: &str, now: DateTime<Utc>) {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.summary == summary) {
        entry.count += 1;
        entry.last_seen = now;
    } else {
        entries.push(SpEntry {
            summary: summary.to_string(),
            count: 1,
            last_seen: now,
        });
    }

    entries.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    if entries.len() > 10 {
        entries.truncate(10);
    }
}

fn upsert_most_recent(entries: &mut Vec<SpEntry>, summary: &str, now: DateTime<Utc>) {
    entries.retain(|entry| entry.summary != summary);
    entries.push(SpEntry {
        summary: summary.to_string(),
        count: 1,
        last_seen: now,
    });
    entries.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    if entries.len() > 10 {
        entries.truncate(10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentStep;
    use tempfile::tempdir;

    #[test]
    fn quarantine_moves_intent_to_failed_queue() {
        let temp = tempdir().unwrap();
        let data_dir = temp.path();
        ensure_data_layout(data_dir).unwrap();

        let queue_dir = data_dir.join("intent/queue");
        let intent_path = queue_dir.join("sample.md");
        std::fs::write(&intent_path, "test").unwrap();

        let moved = quarantine_failed_intent(&intent_path, data_dir).unwrap();
        assert!(!intent_path.exists());
        assert!(moved.exists());
        assert!(moved.starts_with(data_dir.join("intent/queue/failed")));
    }

    fn sample_intent_with_path(path: PathBuf) -> Intent {
        Intent {
            id: Uuid::new_v4(),
            source: "unit-test".to_string(),
            summary: "Write summary".to_string(),
            telos_alignment: 0.9,
            created_at: Utc::now(),
            storage_path: Some(path),
        }
    }

    fn sample_outcome() -> AgentOutcome {
        AgentOutcome {
            steps: vec![AgentStep {
                thought: "Collect context".to_string(),
                action: "summarize_intent".to_string(),
                observation: "Remaining backlog count: 1".to_string(),
            }],
            final_answer: "Done".to_string(),
        }
    }

    #[tokio::test]
    async fn persist_intent_writes_markdown_front_matter() {
        let temp = tempdir().unwrap();
        ensure_data_layout(temp.path()).unwrap();

        let record = persist_intent(
            temp.path(),
            "cli",
            "Launch sequence",
            0.7,
            "## body\ncontent",
        )
        .await
        .unwrap();

        let content = tokio::fs::read_to_string(&record.path).await.unwrap();
        assert!(record.path.starts_with(temp.path().join("intent/inbox")));
        assert!(content.contains("summary: Launch sequence"));
        assert!(content.contains("## body"));
    }

    #[tokio::test]
    async fn append_journal_entry_persists_trace() {
        let temp = tempdir().unwrap();
        ensure_data_layout(temp.path()).unwrap();

        let queue_dir = temp.path().join("intent/queue");
        std::fs::create_dir_all(&queue_dir).unwrap();
        let source_path = queue_dir.join("intent.md");
        std::fs::write(&source_path, "---\nsummary: intent\n---").unwrap();

        let intent = sample_intent_with_path(source_path.clone());
        let outcome = sample_outcome();

        append_journal_entry(temp.path(), &intent, &outcome)
            .await
            .unwrap();

        let now = Utc::now();
        let journal_path = temp
            .path()
            .join("journals")
            .join(format!("{:04}", now.year()))
            .join(format!("{:02}", now.month()))
            .join(format!("{:02}.md", now.day()));

        let entry = tokio::fs::read_to_string(&journal_path).await.unwrap();
        assert!(entry.contains("Final answer: Done"));
        assert!(entry.contains("ReAct trace"));
    }

    #[tokio::test]
    async fn update_sp_index_increments_counts_and_recent() {
        let temp = tempdir().unwrap();
        ensure_data_layout(temp.path()).unwrap();

        let queue_dir = temp.path().join("intent/queue");
        std::fs::create_dir_all(&queue_dir).unwrap();
        let source_path = queue_dir.join("intent.md");
        std::fs::write(&source_path, "---\nsummary: intent\n---").unwrap();

        let intent = sample_intent_with_path(source_path);
        let outcome = sample_outcome();

        update_sp_index(temp.path(), &intent, &outcome)
            .await
            .unwrap();
        update_sp_index(temp.path(), &intent, &outcome)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(temp.path().join("sp/index.json"))
            .await
            .unwrap();
        let persisted: PersistedSpIndex = serde_json::from_str(&content).unwrap();

        assert_eq!(persisted.top_used.len(), 1);
        assert_eq!(persisted.top_used[0].count, 2);
        assert!(
            persisted.top_used[0]
                .summary
                .contains("Write summary ⇒ Done")
        );
        assert_eq!(persisted.most_recent.len(), 1);
        assert!(
            persisted.most_recent[0]
                .summary
                .contains("Write summary ⇒ Done")
        );
    }

    #[test]
    fn sanitize_rejects_traversal_and_accepts_relative() {
        assert!(sanitize_data_relative_path("journals/2025/01/01.md").is_ok());
        assert!(sanitize_data_relative_path("../secret.md").is_err());
        assert!(sanitize_data_relative_path("").is_err());
    }

    #[tokio::test]
    async fn list_tree_and_read_markdown_file() {
        let temp = tempdir().unwrap();
        ensure_data_layout(temp.path()).unwrap();

        let intent_dir = temp.path().join("intent/history");
        std::fs::create_dir_all(&intent_dir).unwrap();
        let file_path = intent_dir.join("example.md");
        tokio::fs::write(&file_path, "# Title\nBody").await.unwrap();

        let tree = list_markdown_tree(temp.path()).unwrap();
        assert_eq!(tree, vec!["intent/history/example.md".to_string()]);

        let relative = sanitize_data_relative_path("intent/history/example.md").unwrap();
        let content = read_markdown_file(temp.path(), &relative)
            .await
            .expect("markdown should be readable");
        assert!(content.contains("# Title"));
    }

    #[tokio::test]
    async fn append_and_read_llm_logs() {
        let temp = tempdir().unwrap();
        ensure_data_layout(temp.path()).unwrap();

        let run_id = Uuid::new_v4();
        let identity = crate::llm::LlmIdentity::new("local_stub", Some("local_stub".to_string()));
        let first = LlmLogEntry::new(
            run_id,
            Utc::now(),
            "THINK",
            "prompt one",
            "response one",
            &identity,
        );
        let second = LlmLogEntry::new(
            run_id,
            Utc::now(),
            "FINAL",
            "prompt two",
            "response two",
            &identity,
        );

        append_llm_logs(temp.path(), &[first.clone(), second.clone()])
            .await
            .unwrap();

        let logs = read_llm_logs(
            temp.path(),
            LlmLogQuery {
                run_id: Some(run_id),
                limit: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(logs.len(), 2);
        assert!(logs.iter().any(|entry| entry.phase == "FINAL"));
        assert!(logs.iter().all(|entry| entry.run_id == run_id));

        let recent_only = read_llm_logs(
            temp.path(),
            LlmLogQuery {
                phase: Some("final".to_string()),
                limit: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(recent_only.len(), 1);
        assert_eq!(recent_only[0].phase, "FINAL");
    }
}
