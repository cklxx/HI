use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{agent::AgentOutcome, tasks::Intent};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum MemoryLevel {
    L1,
    L2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MemoryAnchor {
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub level: MemoryLevel,
    pub summary: String,
    pub details: Vec<String>,
    pub anchors: Vec<MemoryAnchor>,
    pub tags: Vec<String>,
    pub related_intents: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct MemorySnapshotInput {
    pub intent: Intent,
    pub outcome: AgentOutcome,
    pub journal_path: PathBuf,
    pub history_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct MemoryQuery {
    pub level: MemoryLevel,
    pub limit: usize,
    pub since: Option<DateTime<Utc>>,
    pub tag: Option<String>,
}

impl Default for MemoryQuery {
    fn default() -> Self {
        Self {
            level: MemoryLevel::L2,
            limit: 20,
            since: None,
            tag: None,
        }
    }
}

pub async fn ingest_memory_snapshot(
    data_dir: &Path,
    input: MemorySnapshotInput,
) -> anyhow::Result<MemoryEntry> {
    let now = Utc::now();
    let mut anchors = Vec::new();

    if let Some(history) = input.history_path.as_ref() {
        if let Some(anchor) = to_anchor(data_dir, "intent/history", history) {
            anchors.push(anchor);
        }
    }

    if let Some(anchor) = to_anchor(data_dir, "journals", &input.journal_path) {
        anchors.push(anchor);
    }

    let tags = derive_tags(&input.intent);
    let summary = format!(
        "{} ⇒ {}",
        input.intent.summary,
        truncate(&input.outcome.final_answer, 160)
    );

    let mut details = Vec::new();
    details.push(format!("Source: {}", input.intent.source));
    details.push(format!("Final: {}", input.outcome.final_answer));

    if let Some(step) = input.outcome.steps.first() {
        details.push(format!("First observation: {}", step.observation));
    }

    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        level: MemoryLevel::L1,
        summary,
        details,
        anchors,
        tags,
        related_intents: vec![input.intent.id],
        created_at: now,
        updated_at: now,
    };

    persist_l1_entry(data_dir, &entry).await?;
    rebuild_l2_for_day(data_dir, now.date_naive()).await?;

    Ok(entry)
}

pub fn read_memory_entries(
    data_dir: &Path,
    query: MemoryQuery,
) -> anyhow::Result<Vec<MemoryEntry>> {
    match query.level {
        MemoryLevel::L1 => read_l1(data_dir, &query),
        MemoryLevel::L2 => read_l2(data_dir, &query),
    }
}

fn read_l1(data_dir: &Path, query: &MemoryQuery) -> anyhow::Result<Vec<MemoryEntry>> {
    let mut entries = Vec::new();
    let root = data_dir.join("memory/l1");
    if !root.exists() {
        return Ok(entries);
    }

    for entry in WalkDir::new(&root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let content = std::fs::read_to_string(entry.path())
            .with_context(|| format!("reading memory l1 file {:?}", entry.path()))?;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed: MemoryEntry = serde_json::from_str(line)
                .with_context(|| format!("parsing memory l1 entry in {:?}", entry.path()))?;
            if let Some(since) = query.since {
                if parsed.created_at < since {
                    continue;
                }
            }
            if let Some(tag) = query.tag.as_ref() {
                if !parsed
                    .tags
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(tag))
                {
                    continue;
                }
            }
            entries.push(parsed);
        }
    }

    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if entries.len() > query.limit {
        entries.truncate(query.limit);
    }
    Ok(entries)
}

fn read_l2(data_dir: &Path, query: &MemoryQuery) -> anyhow::Result<Vec<MemoryEntry>> {
    let mut entries = Vec::new();
    let root = data_dir.join("memory/l2");
    if !root.exists() {
        return Ok(entries);
    }

    for entry in WalkDir::new(&root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let content = std::fs::read_to_string(entry.path())
            .with_context(|| format!("reading memory l2 file {:?}", entry.path()))?;
        let parsed: MemoryEntry = serde_json::from_str(&content)
            .with_context(|| format!("parsing memory l2 entry in {:?}", entry.path()))?;
        if let Some(since) = query.since {
            if parsed.created_at < since {
                continue;
            }
        }
        if let Some(tag) = query.tag.as_ref() {
            if !parsed
                .tags
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(tag))
            {
                continue;
            }
        }
        entries.push(parsed);
    }

    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if entries.len() > query.limit {
        entries.truncate(query.limit);
    }
    Ok(entries)
}

async fn persist_l1_entry(data_dir: &Path, entry: &MemoryEntry) -> anyhow::Result<()> {
    let date = entry.created_at.date_naive();
    let dir = data_dir
        .join("memory/l1")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()));
    fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{:02}.jsonl", date.day()));

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    let serialized = serde_json::to_string(entry)?;
    file.write_all(serialized.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

async fn rebuild_l2_for_day(data_dir: &Path, date: NaiveDate) -> anyhow::Result<()> {
    let l1_path = data_dir
        .join("memory/l1")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}.jsonl", date.day()));

    if !l1_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&l1_path)
        .await
        .with_context(|| format!("reading l1 entries for rollup {:?}", l1_path))?;

    let mut entries = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: MemoryEntry = serde_json::from_str(line)
            .with_context(|| format!("parsing l1 entry during rollup {:?}", l1_path))?;
        entries.push(entry);
    }

    if entries.is_empty() {
        return Ok(());
    }

    let existing_path = data_dir
        .join("memory/l2")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}.json", date.day()));

    let (previous_id, created_at) = if existing_path.exists() {
        let raw = fs::read_to_string(&existing_path)
            .await
            .with_context(|| format!("reading existing l2 {:?}", existing_path))?;
        let parsed: MemoryEntry = serde_json::from_str(&raw)
            .with_context(|| format!("parsing existing l2 {:?}", existing_path))?;
        (parsed.id, parsed.created_at)
    } else {
        (Uuid::new_v4(), entries[0].created_at)
    };

    let updated_at = Utc::now();
    let summary = format!("{} memories on {}", entries.len(), date.format("%Y-%m-%d"));

    let mut details = Vec::new();
    for entry in entries.iter().take(6) {
        details.push(format!("• {}", entry.summary));
    }

    let mut seen = HashSet::new();
    let mut anchors = Vec::new();
    let mut tags = HashSet::new();
    let mut related = HashSet::new();

    for entry in &entries {
        for anchor in &entry.anchors {
            if seen.insert(anchor.clone()) {
                anchors.push(anchor.clone());
            }
        }
        for tag in &entry.tags {
            tags.insert(tag.clone());
        }
        for intent in &entry.related_intents {
            related.insert(*intent);
        }
    }

    let rollup = MemoryEntry {
        id: previous_id,
        level: MemoryLevel::L2,
        summary,
        details,
        anchors,
        tags: tags.into_iter().collect(),
        related_intents: related.into_iter().collect(),
        created_at,
        updated_at,
    };

    let dir = existing_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("l2 path missing parent"))?;
    fs::create_dir_all(&dir).await?;
    let serialized = serde_json::to_string_pretty(&rollup)?;
    fs::write(&existing_path, serialized.as_bytes()).await?;
    Ok(())
}

fn derive_tags(intent: &Intent) -> Vec<String> {
    let mut tags = HashSet::new();
    tags.insert(intent.source.to_lowercase());
    for token in intent.summary.split_whitespace() {
        let cleaned = token
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if cleaned.len() >= 3 {
            tags.insert(cleaned);
        }
        if tags.len() >= 8 {
            break;
        }
    }
    tags.into_iter().collect()
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut slice = value[..max].to_string();
    slice.push('…');
    slice
}

fn to_anchor(data_dir: &Path, label: &str, path: &Path) -> Option<MemoryAnchor> {
    let relative = path.strip_prefix(data_dir).ok()?;
    Some(MemoryAnchor {
        label: label.to_string(),
        path: relative.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentStep;
    use tempfile::TempDir;

    #[tokio::test]
    async fn ingest_builds_l1_and_l2_records() {
        let temp = TempDir::new().expect("tempdir");
        let data_dir = temp.path();
        fs::create_dir_all(data_dir.join("memory"))
            .await
            .expect("memory dir");

        let intent = Intent {
            id: Uuid::new_v4(),
            source: "telegram".to_string(),
            summary: "Draft weekly report".to_string(),
            telos_alignment: 0.9,
            created_at: Utc::now(),
            storage_path: None,
        };
        let outcome = AgentOutcome {
            steps: vec![AgentStep {
                thought: "review context".to_string(),
                action: "summarize".to_string(),
                observation: "Wrote outline".to_string(),
            }],
            final_answer: "Outlined next steps".to_string(),
        };

        let journal_path = data_dir.join("journals/2025/01/01.md");
        fs::create_dir_all(journal_path.parent().unwrap())
            .await
            .expect("journal parent");
        fs::write(&journal_path, b"stub")
            .await
            .expect("journal file");

        let history_path = data_dir.join("intent/history/intent.md");
        fs::create_dir_all(history_path.parent().unwrap())
            .await
            .expect("history parent");
        fs::write(&history_path, b"history")
            .await
            .expect("history file");

        ingest_memory_snapshot(
            data_dir,
            MemorySnapshotInput {
                intent: intent.clone(),
                outcome: outcome.clone(),
                journal_path: journal_path.clone(),
                history_path: Some(history_path.clone()),
            },
        )
        .await
        .expect("ingest");

        let l1_entries = read_memory_entries(
            data_dir,
            MemoryQuery {
                level: MemoryLevel::L1,
                limit: 10,
                since: None,
                tag: None,
            },
        )
        .expect("read l1");
        assert_eq!(l1_entries.len(), 1);
        assert!(
            l1_entries[0]
                .anchors
                .iter()
                .any(|anchor| anchor.path.contains("intent/history"))
        );

        let l2_entries = read_memory_entries(
            data_dir,
            MemoryQuery {
                level: MemoryLevel::L2,
                limit: 10,
                since: None,
                tag: None,
            },
        )
        .expect("read l2");
        assert_eq!(l2_entries.len(), 1);
        assert_eq!(l2_entries[0].level, MemoryLevel::L2);
        assert!(!l2_entries[0].details.is_empty());
    }
}
