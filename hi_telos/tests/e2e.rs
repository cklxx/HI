use std::{fs, sync::Arc, time::Duration};

use anyhow::Result;
use hi_telos::{agent::AgentRuntime, config::AppConfig, orchestrator, state::AppContext, storage};
use tempfile::TempDir;
use tokio::time::{sleep, timeout};

#[tokio::test]
async fn beat_ingests_intent_and_writes_journal() -> Result<()> {
    let tmp = TempDir::new()?;
    let root = tmp.path();

    fs::create_dir_all(root.join("config"))?;
    fs::write(
        root.join("config/beat.yml"),
        "interval_minutes: 5\nintent_threshold: 0.4\n",
    )?;
    fs::write(
        root.join("config/agent.yml"),
        "max_react_steps: 1\npersona: TelosOps\n",
    )?;
    fs::write(root.join("config/llm.yml"), "provider: local_stub\n")?;

    unsafe {
        std::env::set_var("HI_APP_ROOT", root);
        std::env::set_var("HI_SERVER_BIND", "127.0.0.1:0");
    }

    let config = AppConfig::load()?;
    let agent_runtime = AgentRuntime::from_app_config(&config)?;
    let data_dir = config.data_dir.clone();
    let ctx = AppContext::new(config, Arc::new(agent_runtime));

    let (handle, join) = orchestrator::spawn(ctx.clone());

    storage::persist_intent(
        &data_dir,
        "tester",
        "Process inbox intent",
        0.9,
        "# Body\nCheck intent flow",
    )
    .await?;

    let history_dir = data_dir.join("intent/history");

    // Give the orchestrator loop time to start before requesting a beat.
    sleep(Duration::from_millis(50)).await;
    handle.request_beat().await?;

    timeout(Duration::from_secs(2), async {
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
        "intent should be archived after processing"
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
        "journal should capture agent final answer"
    );

    let sp_index = storage::load_sp_index(&data_dir).await?;
    assert!(
        sp_index
            .top_used
            .iter()
            .any(|entry| entry.contains("TelosOps completed the plan")),
        "SP index should track top used intents"
    );
    assert!(
        sp_index
            .most_recent
            .iter()
            .any(|entry| entry.contains("TelosOps completed the plan")),
        "SP index should include most recent intents"
    );

    let logs = storage::read_llm_logs(&data_dir, storage::LlmLogQuery::default()).await?;
    assert!(
        logs.iter().any(|entry| entry.phase == "FINAL"),
        "LLM logs should include final phase entries"
    );
    assert!(
        logs.iter()
            .any(|entry| entry.prompt.contains("# Phase: FINAL")),
        "LLM logs should capture prompts"
    );

    ctx.request_shutdown();
    let _ = join.await;

    unsafe {
        std::env::remove_var("HI_APP_ROOT");
        std::env::remove_var("HI_SERVER_BIND");
    }

    Ok(())
}
