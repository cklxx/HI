use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs;

const STRUCTURED_TEXT_HISTORY_LIMIT: usize = 20;
const HISTORY_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%S%6fZ";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructuredTextHistoryFilters {
    pub since: Option<DateTime<Utc>>,
    pub note_query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedStructuredTextPreview {
    pub content: StructuredContent,
    pub note: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Structured section content surfaced to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredSection {
    pub heading: String,
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub children: Vec<StructuredSection>,
}

/// Structured content payload returned by the preview endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredContent {
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub sections: Vec<StructuredSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StructuredTextSnapshot {
    pub content: StructuredContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl StructuredContent {
    /// Convenience helper for generating the existing inline fallback payload.
    pub fn mock_payload() -> Self {
        Self {
            title: "Telos Operator Handbook".to_string(),
            summary: "Mock content that mirrors a full operator runbook so the UI can exercise typography, metadata badges, and deep nesting.".to_string(),
            sections: vec![
                StructuredSection {
                    heading: "1 · Environment Bootstrapping".to_string(),
                    body: vec![
                        "Install the published fixtures to hydrate inbox intents, configuration files, and baseline journals.".to_string(),
                        "Export HI_APP_ROOT before starting the binary so relative paths resolve to the mounted data directory.".to_string(),
                    ],
                    children: vec![
                        StructuredSection {
                            heading: "Configuration Files".to_string(),
                            body: vec![
                                "config/beat.yml governs cadence, intent thresholds, and retry behaviour.".to_string(),
                                "config/agent.yml defines the persona prompts and max ReAct turns.".to_string(),
                            ],
                            children: vec![StructuredSection {
                                heading: "LLM Overrides".to_string(),
                                body: vec![
                                    "Swap config/llm.yml to the OpenAI profile when a real model is available.".to_string(),
                                    "Set OPENAI_API_KEY in the environment before launching the orchestrator.".to_string(),
                                ],
                                children: vec![],
                            }],
                        },
                        StructuredSection {
                            heading: "Operator Checklist".to_string(),
                            body: vec![
                                "Run cargo run -p hi_telos --bin bootstrap_fixtures -- ./tmp/hi-telos-core.".to_string(),
                                "Confirm intent inbox entries exist under data/intent/inbox before triggering Beats.".to_string(),
                                "Open docs/work_acceptance_plan.md to review current delivery scope.".to_string(),
                            ],
                            children: vec![],
                        },
                    ],
                },
                StructuredSection {
                    heading: "2 · Heartbeat Execution".to_string(),
                    body: vec![
                        "Beats pull intents from inbox, route them through the ReAct loop, and persist artefacts to journals and SP indexes.".to_string(),
                        "Mock payloads expose THINK/ACT/OBSERVE sections so the UI can preview accordion and timeline styling.".to_string(),
                    ],
                    children: vec![
                        StructuredSection {
                            heading: "Scheduler".to_string(),
                            body: vec![
                                "Each beat iteration evaluates queue depth and skips when nothing actionable is present.".to_string(),
                                "Failed intents are shunted into intent/queue/failed with exponential backoff metadata.".to_string(),
                            ],
                            children: vec![],
                        },
                        StructuredSection {
                            heading: "Agent Loop".to_string(),
                            body: vec![
                                "The orchestrator emits THINK, ACT, and OBSERVE events, which downstream dashboards render as timeline items.".to_string(),
                                "Final answers are appended to journals/YYYY/MM/DD.md alongside the originating intent summary.".to_string(),
                            ],
                            children: vec![StructuredSection {
                                heading: "Storage Touchpoints".to_string(),
                                body: vec![
                                    "SP index highlights top_used and most_recent resolutions for quick recall.".to_string(),
                                    "LLM logs capture prompts/responses with metadata (model, run_id) for auditing.".to_string(),
                                ],
                                children: vec![],
                            }],
                        },
                        StructuredSection {
                            heading: "Observability".to_string(),
                            body: vec![
                                "/ui/logs streams combined LLM transcripts, SP snapshots, and memory rollups.".to_string(),
                                "/ui/messages surfaces inbox, queue, and archive slices for a quick health check.".to_string(),
                            ],
                            children: vec![],
                        },
                    ],
                },
                StructuredSection {
                    heading: "3 · Appendix".to_string(),
                    body: vec![
                        "Reference APIs and mock utilities that unblock front-end development without waiting for live traffic.".to_string(),
                    ],
                    children: vec![
                        StructuredSection {
                            heading: "Mock Endpoints".to_string(),
                            body: vec![
                                "GET /api/mock/text_structure returns the latest structured preview payload.".to_string(),
                                "POST /api/mock/text_structure accepts either a full content object or {\"content\": ..., \"note\": \"...\"}.".to_string(),
                                "DELETE /api/mock/text_structure clears overrides so the inline Rust payload is served again.".to_string(),
                            ],
                            children: vec![StructuredSection {
                                heading: "History Controls".to_string(),
                                body: vec![
                                    "GET /api/mock/text_structure/history?limit=5 lists the most recent persisted drafts.".to_string(),
                                    "POST /api/mock/text_structure/history/{id}/restore promotes an old draft back to the preview state.".to_string(),
                                ],
                                children: vec![],
                            }],
                        },
                        StructuredSection {
                            heading: "Design Tokens".to_string(),
                            body: vec![
                                "Use monospace typography, neon accents, and dashed separators to match the Telos console aesthetic.".to_string(),
                                "Nested sections should indent with subtle dashed lines rather than bullets for clarity.".to_string(),
                            ],
                            children: vec![],
                        },
                    ],
                },
            ],
        }
    }
}

/// Attempt to load a structured text preview from disk.
///
/// The preview is stored in `<data_dir>/mock/text_structure.json`. Missing files
/// are treated as a soft failure and return `Ok(None)` so the caller can fall
/// back to the inline payload while still logging the issue. Any other IO or
/// parsing errors are surfaced to the caller for observability.
pub async fn load_structured_text_preview(
    data_dir: &Path,
) -> Result<Option<LoadedStructuredTextPreview>> {
    let path = data_dir.join("mock/text_structure.json");
    match fs::read_to_string(&path).await {
        Ok(raw) => {
            let metadata = fs::metadata(&path).await.ok();
            let updated_at = metadata
                .and_then(|meta| meta.modified().ok())
                .map(DateTime::<Utc>::from);
            let snapshot = parse_snapshot(&raw)
                .with_context(|| format!("parsing structured text preview at {:?}", path))?;
            Ok(Some(LoadedStructuredTextPreview {
                content: snapshot.content,
                note: snapshot.note,
                updated_at,
            }))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Persist a structured text preview to disk so subsequent calls to the mock
/// endpoint return the freshly authored content.
pub async fn save_structured_text_preview(
    data_dir: &Path,
    payload: &StructuredContent,
    note: Option<&str>,
) -> Result<()> {
    let mock_dir = data_dir.join("mock");
    fs::create_dir_all(&mock_dir)
        .await
        .with_context(|| format!("creating mock directory at {:?}", mock_dir))?;

    let snapshot = StructuredTextSnapshot {
        content: payload.clone(),
        note: note.map(str::to_string),
    };
    let serialized =
        serde_json::to_vec_pretty(&snapshot).context("serializing structured text preview")?;
    let path = mock_dir.join("text_structure.json");
    fs::write(&path, serialized)
        .await
        .with_context(|| format!("writing structured text preview at {:?}", path))?;

    append_structured_text_history(&mock_dir, payload, note).await?;

    Ok(())
}

pub async fn delete_structured_text_preview(data_dir: &Path) -> Result<()> {
    let path = data_dir.join("mock/text_structure.json");
    match fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredTextHistoryEntry {
    pub id: String,
    pub saved_at: DateTime<Utc>,
    pub content: StructuredContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub async fn list_structured_text_history(
    data_dir: &Path,
    limit: usize,
    filters: Option<&StructuredTextHistoryFilters>,
) -> Result<Vec<StructuredTextHistoryEntry>> {
    let history_dir = data_dir.join("mock/text_structure_history");
    if !history_dir.exists() {
        return Ok(Vec::new());
    }

    let mut dir = fs::read_dir(&history_dir)
        .await
        .with_context(|| format!("reading structured text history at {:?}", history_dir))?;

    let mut entries = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        let path = entry.path();
        if !entry.file_type().await?.is_file() {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };

        let saved_at = match parse_history_id(stem) {
            Ok(ts) => ts,
            Err(_) => continue,
        };

        let raw = fs::read_to_string(&path)
            .await
            .with_context(|| format!("reading structured text history file {:?}", path))?;
        let snapshot = parse_snapshot(&raw)
            .with_context(|| format!("parsing structured text history file {:?}", path))?;

        entries.push(StructuredTextHistoryEntry {
            id: stem.to_string(),
            saved_at,
            content: snapshot.content,
            note: snapshot.note,
        });
    }

    if let Some(filters) = filters {
        if let Some(since) = filters.since.as_ref() {
            entries.retain(|entry| entry.saved_at >= *since);
        }

        if let Some(needle) = filters.note_query.as_ref().and_then(|query| {
            let trimmed = query.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_lowercase())
            }
        }) {
            entries.retain(|entry| entry_contains_query(entry, &needle));
        }
    }

    entries.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    let limit = if limit == 0 {
        STRUCTURED_TEXT_HISTORY_LIMIT
    } else {
        limit
    };
    if entries.len() > limit {
        entries.truncate(limit);
    }

    Ok(entries)
}

async fn append_structured_text_history(
    mock_dir: &Path,
    payload: &StructuredContent,
    note: Option<&str>,
) -> Result<()> {
    let history_dir = mock_dir.join("text_structure_history");
    fs::create_dir_all(&history_dir)
        .await
        .with_context(|| format!("creating structured text history dir at {:?}", history_dir))?;

    let now = Utc::now();
    let timestamp = now.format(HISTORY_TIMESTAMP_FORMAT).to_string();
    let history_path = history_dir.join(format!("{}.json", timestamp));
    let snapshot = StructuredTextSnapshot {
        content: payload.clone(),
        note: note.map(str::to_string),
    };
    let serialized = serde_json::to_vec_pretty(&snapshot)
        .context("serializing structured text history entry")?;
    fs::write(&history_path, serialized)
        .await
        .with_context(|| {
            format!(
                "writing structured text history entry at {:?}",
                history_path
            )
        })?;

    prune_structured_text_history(&history_dir, STRUCTURED_TEXT_HISTORY_LIMIT).await?;

    Ok(())
}

async fn prune_structured_text_history(history_dir: &Path, limit: usize) -> Result<()> {
    let mut entries = fs::read_dir(history_dir)
        .await
        .with_context(|| format!("reading structured text history at {:?}", history_dir))?;

    let mut indexed: Vec<(DateTime<Utc>, std::path::PathBuf)> = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !entry.file_type().await?.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if let Ok(ts) = parse_history_id(stem) {
            indexed.push((ts, path));
        }
    }

    indexed.sort_by(|a, b| b.0.cmp(&a.0));
    if indexed.len() <= limit {
        return Ok(());
    }

    for (_, path) in indexed.into_iter().skip(limit) {
        if let Err(err) = fs::remove_file(&path).await
            && err.kind() != std::io::ErrorKind::NotFound
        {
            return Err(err.into());
        }
    }

    Ok(())
}

fn parse_history_id(id: &str) -> Result<DateTime<Utc>> {
    let trimmed = id
        .strip_suffix('Z')
        .ok_or_else(|| anyhow::anyhow!("invalid structured text history id: {id}"))?;
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y%m%dT%H%M%S%6f")
        .with_context(|| format!("invalid structured text history id: {id}"))?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
}

pub async fn load_structured_text_history_entry(
    data_dir: &Path,
    id: &str,
) -> Result<Option<StructuredTextHistoryEntry>> {
    let history_dir = data_dir.join("mock/text_structure_history");
    if !history_dir.exists() {
        return Ok(None);
    }

    let saved_at = parse_history_id(id)?;
    let path = history_dir.join(format!("{}.json", id));

    match fs::read_to_string(&path).await {
        Ok(raw) => {
            let snapshot = parse_snapshot(&raw)
                .with_context(|| format!("parsing structured text history file {:?}", path))?;
            Ok(Some(StructuredTextHistoryEntry {
                id: id.to_string(),
                saved_at,
                content: snapshot.content,
                note: snapshot.note,
            }))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub async fn restore_structured_text_preview_from_history(
    data_dir: &Path,
    id: &str,
) -> Result<bool> {
    match load_structured_text_history_entry(data_dir, id).await? {
        Some(entry) => {
            save_structured_text_preview(data_dir, &entry.content, entry.note.as_deref()).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn parse_snapshot(raw: &str) -> Result<StructuredTextSnapshot> {
    match serde_json::from_str::<StructuredTextSnapshot>(raw) {
        Ok(snapshot) => Ok(snapshot),
        Err(_) => {
            let content: StructuredContent =
                serde_json::from_str(raw).context("parsing legacy structured text snapshot")?;
            Ok(StructuredTextSnapshot {
                content,
                note: None,
            })
        }
    }
}

fn entry_contains_query(entry: &StructuredTextHistoryEntry, needle: &str) -> bool {
    if let Some(note) = entry.note.as_ref()
        && note.to_lowercase().contains(needle)
    {
        return true;
    }

    if entry.content.title.to_lowercase().contains(needle)
        || entry.content.summary.to_lowercase().contains(needle)
        || entry
            .content
            .sections
            .iter()
            .any(|section| section_contains_query(section, needle))
    {
        return true;
    }

    false
}

fn section_contains_query(section: &StructuredSection, needle: &str) -> bool {
    if section.heading.to_lowercase().contains(needle)
        || section
            .body
            .iter()
            .any(|line| line.to_lowercase().contains(needle))
    {
        return true;
    }

    section
        .children
        .iter()
        .any(|child| section_contains_query(child, needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn load_structured_text_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let payload = load_structured_text_preview(data_dir).await.unwrap();
        assert!(payload.is_none());
    }

    #[tokio::test]
    async fn load_structured_text_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let expected = StructuredContent {
            title: "Example".to_string(),
            summary: "Summary".to_string(),
            sections: vec![StructuredSection {
                heading: "Heading".to_string(),
                body: vec!["Body".to_string()],
                children: vec![],
            }],
        };

        let path = data_dir.join("mock");
        tokio::fs::create_dir_all(&path).await.unwrap();
        tokio::fs::write(
            path.join("text_structure.json"),
            serde_json::to_string(&expected).unwrap(),
        )
        .await
        .unwrap();

        let payload = load_structured_text_preview(data_dir).await.unwrap();
        let preview = payload.expect("preview");
        assert_eq!(preview.content, expected);
        assert!(preview.updated_at.is_some());
    }

    #[tokio::test]
    async fn save_structured_text_writes_file() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let content = StructuredContent {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            sections: vec![StructuredSection {
                heading: "Heading".to_string(),
                body: vec!["Body".to_string()],
                children: vec![],
            }],
        };

        save_structured_text_preview(data_dir, &content, None)
            .await
            .expect("save structured text");

        let persisted = load_structured_text_preview(data_dir)
            .await
            .expect("load structured text")
            .expect("some");
        assert_eq!(persisted.content, content);
        assert!(persisted.note.is_none());
        assert!(persisted.updated_at.is_some());
    }

    #[tokio::test]
    async fn save_structured_text_creates_history_entry() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let content = StructuredContent {
            title: "History".to_string(),
            summary: "History summary".to_string(),
            sections: vec![],
        };

        save_structured_text_preview(data_dir, &content, Some("first draft"))
            .await
            .expect("save structured text");

        let history_dir = data_dir.join("mock/text_structure_history");
        assert!(history_dir.exists());
        let mut entries = tokio::fs::read_dir(&history_dir)
            .await
            .expect("history dir");
        let mut count = 0;
        while entries.next_entry().await.expect("entry").is_some() {
            count += 1;
        }
        assert_eq!(count, 1);

        let history_entries = list_structured_text_history(data_dir, 10, None)
            .await
            .expect("list history");
        assert_eq!(history_entries.len(), 1);
        assert_eq!(history_entries[0].note.as_deref(), Some("first draft"));
    }

    #[tokio::test]
    async fn delete_structured_text_preview_removes_file() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let content = StructuredContent {
            title: "Title".to_string(),
            summary: "Summary".to_string(),
            sections: vec![StructuredSection {
                heading: "Heading".to_string(),
                body: vec!["Body".to_string()],
                children: vec![],
            }],
        };

        save_structured_text_preview(data_dir, &content, None)
            .await
            .expect("save structured text");

        delete_structured_text_preview(data_dir)
            .await
            .expect("delete structured text");

        let payload = load_structured_text_preview(data_dir)
            .await
            .expect("load structured text");
        assert!(payload.is_none());
    }

    #[tokio::test]
    async fn list_structured_text_history_sorts_by_timestamp() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let history_dir = data_dir.join("mock/text_structure_history");
        tokio::fs::create_dir_all(&history_dir)
            .await
            .expect("history dir");

        let older = StructuredContent {
            title: "Older".to_string(),
            summary: "Older summary".to_string(),
            sections: vec![],
        };
        let newer = StructuredContent {
            title: "Newer".to_string(),
            summary: "Newer summary".to_string(),
            sections: vec![],
        };

        tokio::fs::write(
            history_dir.join("20240101T000000000000Z.json"),
            serde_json::to_vec_pretty(&older).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            history_dir.join("20240201T000000000000Z.json"),
            serde_json::to_vec_pretty(&newer).unwrap(),
        )
        .await
        .unwrap();

        let entries = list_structured_text_history(data_dir, 10, None)
            .await
            .expect("list history");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "20240201T000000000000Z");
        assert_eq!(entries[0].content.title, "Newer");
        assert_eq!(entries[1].id, "20240101T000000000000Z");
        assert_eq!(entries[1].content.title, "Older");
    }

    #[tokio::test]
    async fn list_structured_text_history_applies_filters() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let history_dir = data_dir.join("mock/text_structure_history");
        tokio::fs::create_dir_all(&history_dir)
            .await
            .expect("history dir");

        let snapshots = vec![
            (
                "20240101T000000000000Z",
                StructuredTextSnapshot {
                    content: StructuredContent {
                        title: "Alpha Title".to_string(),
                        summary: "Alpha Summary".to_string(),
                        sections: vec![StructuredSection {
                            heading: "Alpha Heading".to_string(),
                            body: vec!["Alpha body paragraph".to_string()],
                            children: vec![],
                        }],
                    },
                    note: Some("Alpha note".to_string()),
                },
            ),
            (
                "20240201T000000000000Z",
                StructuredTextSnapshot {
                    content: StructuredContent {
                        title: "Beta Title".to_string(),
                        summary: "Beta Summary".to_string(),
                        sections: vec![StructuredSection {
                            heading: "Beta Heading".to_string(),
                            body: vec!["Contains important beta checklist".to_string()],
                            children: vec![],
                        }],
                    },
                    note: Some("Beta release".to_string()),
                },
            ),
            (
                "20240315T120000000000Z",
                StructuredTextSnapshot {
                    content: StructuredContent {
                        title: "Gamma Title".to_string(),
                        summary: "Highlights gamma timeline".to_string(),
                        sections: vec![StructuredSection {
                            heading: "Gamma Overview".to_string(),
                            body: vec!["Gamma body mentions milestones".to_string()],
                            children: vec![],
                        }],
                    },
                    note: None,
                },
            ),
        ];

        for (file, snapshot) in snapshots {
            let path = history_dir.join(format!("{file}.json"));
            tokio::fs::write(&path, serde_json::to_vec_pretty(&snapshot).unwrap())
                .await
                .expect("write snapshot");
        }

        let since = chrono::DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let since_filter = StructuredTextHistoryFilters {
            since: Some(since),
            note_query: None,
        };
        let filtered = list_structured_text_history(data_dir, 10, Some(&since_filter))
            .await
            .expect("list history since");
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .all(|entry| entry.id != "20240101T000000000000Z")
        );

        let note_filter = StructuredTextHistoryFilters {
            since: None,
            note_query: Some("beta".to_string()),
        };
        let filtered = list_structured_text_history(data_dir, 10, Some(&note_filter))
            .await
            .expect("list history by note");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "20240201T000000000000Z");

        let combined_filter = StructuredTextHistoryFilters {
            since: Some(
                chrono::DateTime::parse_from_rfc3339("2024-03-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            note_query: Some("milestones".to_string()),
        };
        let filtered = list_structured_text_history(data_dir, 10, Some(&combined_filter))
            .await
            .expect("list history combined filters");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "20240315T120000000000Z");
    }

    #[tokio::test]
    async fn load_structured_text_history_entry_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        save_structured_text_preview(
            data_dir,
            &StructuredContent {
                title: "Snapshot".to_string(),
                summary: "Snapshot summary".to_string(),
                sections: vec![],
            },
            Some("snapshot note"),
        )
        .await
        .expect("save structured text");

        let entries = list_structured_text_history(data_dir, 1, None)
            .await
            .expect("history entries");
        let entry = load_structured_text_history_entry(data_dir, &entries[0].id)
            .await
            .expect("load entry")
            .expect("some entry");

        assert_eq!(entry.id, entries[0].id);
        assert_eq!(entry.content.title, "Snapshot");
        assert_eq!(entry.note.as_deref(), Some("snapshot note"));
    }

    #[tokio::test]
    async fn restore_structured_text_preview_from_history_replays_content() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        save_structured_text_preview(
            data_dir,
            &StructuredContent {
                title: "First".to_string(),
                summary: "First summary".to_string(),
                sections: vec![],
            },
            Some("first note"),
        )
        .await
        .expect("save first");

        let entries = list_structured_text_history(data_dir, 1, None)
            .await
            .expect("history entries");
        let first_id = entries[0].id.clone();

        save_structured_text_preview(
            data_dir,
            &StructuredContent {
                title: "Second".to_string(),
                summary: "Second summary".to_string(),
                sections: vec![],
            },
            None,
        )
        .await
        .expect("save second");

        let restored = restore_structured_text_preview_from_history(data_dir, &first_id)
            .await
            .expect("restore");
        assert!(restored);

        let preview = load_structured_text_preview(data_dir)
            .await
            .expect("preview")
            .expect("some preview");
        assert_eq!(preview.content.title, "First");
        assert_eq!(preview.note.as_deref(), Some("first note"));
    }

    #[tokio::test]
    async fn save_structured_text_with_note_persists_to_preview_and_history() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let content = StructuredContent {
            title: "With Note".to_string(),
            summary: "Summary".to_string(),
            sections: vec![],
        };

        save_structured_text_preview(data_dir, &content, Some("author note"))
            .await
            .expect("save structured text");

        let preview = load_structured_text_preview(data_dir)
            .await
            .expect("preview")
            .expect("some preview");
        assert_eq!(preview.note.as_deref(), Some("author note"));

        let history = list_structured_text_history(data_dir, 5, None)
            .await
            .expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].note.as_deref(), Some("author note"));
    }
}
