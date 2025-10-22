use std::{fs, sync::Arc, time::Duration};

use anyhow::Result;
use hi_telos::{
    agent::AgentRuntime,
    config::AppConfig,
    orchestrator,
    server::{self, ServerState},
    state::AppContext,
    storage::{self, StructuredContent, StructuredSection},
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;
use tokio::{
    net::TcpListener,
    time::{sleep, timeout},
};

mod common;

#[tokio::test]
async fn beat_ingests_intent_and_writes_journal() -> Result<()> {
    let tmp = TempDir::new()?;
    let root = tmp.path();
    let fixture_root = common::install_core_fixture(root)?;

    unsafe {
        std::env::set_var("HI_APP_ROOT", &fixture_root);
        std::env::set_var("HI_SERVER_BIND", "127.0.0.1:0");
    }

    let config = AppConfig::load()?;
    let agent_runtime = AgentRuntime::from_app_config(&config)?;
    let data_dir = config.data_dir.clone();
    let ctx = AppContext::new(config, Arc::new(agent_runtime));

    let (handle, join) = orchestrator::spawn(ctx.clone());

    // Give the orchestrator loop time to start before requesting a beat.
    sleep(Duration::from_millis(50)).await;
    handle.request_beat().await?;

    let history_dir = data_dir.join("intent/history");

    timeout(Duration::from_secs(5), async {
        loop {
            match fs::read_dir(&history_dir) {
                Ok(mut entries) => {
                    if entries.next().is_some() {
                        break;
                    }
                }
                Err(err) => return Err(anyhow::anyhow!(err)),
            }
            sleep(Duration::from_millis(50)).await;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await??;

    let history_entries: Vec<_> = fs::read_dir(&history_dir)?.collect();
    assert_eq!(
        history_entries.len(),
        1,
        "intent should be archived after processing",
    );

    let inbox_dir = data_dir.join("intent/inbox");
    let inbox_files = storage::list_markdown_files(&inbox_dir);
    assert!(inbox_files.is_empty(), "inbox should be empty");

    let journal_dir = data_dir.join("journals");
    let journal_files = storage::list_markdown_files(&journal_dir);
    assert_eq!(journal_files.len(), 1, "one journal entry expected");
    let journal_content = tokio::fs::read_to_string(&journal_files[0]).await?;
    assert!(
        journal_content
            .contains("Final answer: TelosOps completed the plan for 'Process inbox intent'"),
        "journal should capture agent final answer",
    );

    let sp_index = storage::load_sp_index(&data_dir).await?;
    assert!(
        sp_index
            .top_used
            .iter()
            .any(|entry| entry.contains("TelosOps completed the plan")),
        "SP index should track top used intents",
    );
    assert!(
        sp_index
            .most_recent
            .iter()
            .any(|entry| entry.contains("TelosOps completed the plan")),
        "SP index should include most recent intents",
    );

    let logs = storage::read_llm_logs(&data_dir, storage::LlmLogQuery::default()).await?;
    assert!(
        logs.iter().any(|entry| entry.phase == "FINAL"),
        "LLM logs should include final phase entries",
    );
    assert!(
        logs.iter()
            .any(|entry| entry.prompt.contains("# Phase: FINAL")),
        "LLM logs should capture prompts",
    );

    ctx.request_shutdown();
    let _ = join.await;

    unsafe {
        std::env::remove_var("HI_APP_ROOT");
        std::env::remove_var("HI_SERVER_BIND");
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct TextStructurePreview {
    title: String,
    source: String,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TextStructureHistory {
    entries: Vec<storage::StructuredTextHistoryEntry>,
}

#[tokio::test]
async fn text_structure_mock_flow_via_http() -> Result<()> {
    let tmp = TempDir::new()?;
    let root = tmp.path();
    let fixture_root = common::install_core_fixture(root)?;

    unsafe {
        std::env::set_var("HI_APP_ROOT", &fixture_root);
        std::env::set_var("HI_SERVER_BIND", "127.0.0.1:0");
    }

    let config = AppConfig::load()?;
    let agent_runtime = AgentRuntime::from_app_config(&config)?;
    let ctx = AppContext::new(config, Arc::new(agent_runtime));

    let (orchestrator_handle, orchestrator_join) = orchestrator::spawn(ctx.clone());
    let state = ServerState::new(ctx.clone(), orchestrator_handle);

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server = tokio::spawn(server::serve_with_listener(listener, state));

    let client = Client::new();
    let base_url = format!("http://{}", addr);

    let mut attempts = 0;
    loop {
        match client.get(format!("{}/healthz", base_url)).send().await {
            Ok(response) if response.status().is_success() => break,
            _ if attempts > 20 => {
                anyhow::bail!("server did not become ready in time");
            }
            _ => {
                attempts += 1;
                sleep(Duration::from_millis(50)).await;
            }
        }
    }

    let preview: TextStructurePreview = client
        .get(format!("{}/api/mock/text_structure", base_url))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(preview.source, "file");
    assert_eq!(preview.note.as_deref(), Some("Seeded Telos preview"));

    let updated_content = StructuredContent {
        title: "E2E Title".to_string(),
        summary: "Updated via e2e test".to_string(),
        sections: vec![StructuredSection {
            heading: "Section".to_string(),
            body: vec!["Line".to_string()],
            children: vec![],
        }],
    };

    let update_payload = json!({
        "content": updated_content,
        "note": "Updated via e2e test",
    });

    let update_response = client
        .post(format!("{}/api/mock/text_structure", base_url))
        .json(&update_payload)
        .send()
        .await?;
    assert_eq!(update_response.status(), reqwest::StatusCode::NO_CONTENT);

    let updated_preview: TextStructurePreview = client
        .get(format!("{}/api/mock/text_structure", base_url))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(updated_preview.title, "E2E Title");
    assert_eq!(
        updated_preview.note.as_deref(),
        Some("Updated via e2e test")
    );

    let history: TextStructureHistory = client
        .get(format!(
            "{}/api/mock/text_structure/history?limit=10",
            base_url
        ))
        .send()
        .await?
        .json()
        .await?;
    assert!(!history.entries.is_empty());
    assert_eq!(
        history.entries[0].note.as_deref(),
        Some("Updated via e2e test")
    );

    let restore_target = history
        .entries
        .iter()
        .find(|entry| entry.note.as_deref() == Some("Initial fixture snapshot"))
        .expect("fixture snapshot should be present");

    let restore_response = client
        .post(format!(
            "{}/api/mock/text_structure/history/{}/restore",
            base_url, restore_target.id
        ))
        .send()
        .await?;
    assert_eq!(restore_response.status(), reqwest::StatusCode::NO_CONTENT);

    let restored_preview: TextStructurePreview = client
        .get(format!("{}/api/mock/text_structure", base_url))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(restored_preview.title, "Fixture Snapshot");
    assert_eq!(
        restored_preview.note.as_deref(),
        Some("Initial fixture snapshot")
    );

    let delete_response = client
        .delete(format!("{}/api/mock/text_structure", base_url))
        .send()
        .await?;
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let reset_preview: TextStructurePreview = client
        .get(format!("{}/api/mock/text_structure", base_url))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(reset_preview.source, "inline");
    assert!(reset_preview.note.is_none());

    ctx.request_shutdown();
    let _ = orchestrator_join.await;
    server.await??;

    unsafe {
        std::env::remove_var("HI_APP_ROOT");
        std::env::remove_var("HI_SERVER_BIND");
    }

    Ok(())
}
