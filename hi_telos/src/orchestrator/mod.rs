use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use tokio::{
    select,
    sync::mpsc::{self, Sender},
    task::JoinHandle,
    time::{interval, sleep},
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{agent::AgentInput, state::AppContext, storage, tasks::Intent};

const STORAGE_RETRY_ATTEMPTS: usize = 3;
const STORAGE_RETRY_DELAY_MS: u64 = 200;
const INTENT_REQUEUE_ATTEMPTS: u8 = 3;

#[derive(Debug)]
pub enum OrchestratorCommand {
    RequestBeat,
}

#[derive(Clone)]
pub struct OrchestratorHandle {
    tx: Sender<OrchestratorCommand>,
}

impl OrchestratorHandle {
    pub async fn request_beat(&self) -> anyhow::Result<()> {
        self.tx
            .send(OrchestratorCommand::RequestBeat)
            .await
            .map_err(|err| anyhow::anyhow!("orchestrator shutdown: {err}"))
    }
}

pub struct BeatOrchestrator {
    ctx: AppContext,
    cmd_rx: mpsc::Receiver<OrchestratorCommand>,
}

impl BeatOrchestrator {
    pub fn new(ctx: AppContext, cmd_rx: mpsc::Receiver<OrchestratorCommand>) -> Self {
        Self { ctx, cmd_rx }
    }

    async fn process_intent(&self, intent: &Intent) -> anyhow::Result<()> {
        let backlog_size = {
            let intents = self.ctx.intents();
            let queue = intents.read();
            queue.len()
        };

        let agent = self.ctx.agent();
        let run = agent
            .run_react(AgentInput {
                intent: intent.clone(),
                backlog_size,
            })
            .await?;
        let outcome = run.outcome.clone();
        let llm_logs = run.llm_logs.clone();

        let config = self.ctx.config();
        let data_dir = config.data_dir.clone();
        drop(config);

        self.run_with_retry(&intent.summary, "llm_logs", || {
            let data_dir = data_dir.clone();
            let llm_logs = llm_logs.clone();
            async move { storage::append_llm_logs(&data_dir, &llm_logs).await }
        })
        .await?;

        self.run_with_retry(&intent.summary, "journal", || {
            let data_dir = data_dir.clone();
            let intent = intent.clone();
            let outcome = outcome.clone();
            async move { storage::append_journal_entry(&data_dir, &intent, &outcome).await }
        })
        .await?;

        self.run_with_retry(&intent.summary, "sp_index", || {
            let data_dir = data_dir.clone();
            let intent = intent.clone();
            let outcome = outcome.clone();
            async move { storage::update_sp_index(&data_dir, &intent, &outcome).await }
        })
        .await?;

        self.run_with_retry(&intent.summary, "archive", || {
            let data_dir = data_dir.clone();
            let intent = intent.clone();
            async move { storage::archive_intent(&intent, &data_dir).await }
        })
        .await?;

        info!(intent = %intent.summary, final = %outcome.final_answer, "beat handled");
        Ok(())
    }

    async fn run_with_retry<F, Fut>(
        &self,
        summary: &str,
        stage: &'static str,
        mut operation: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = anyhow::Result<()>> + Send,
    {
        let mut remaining = STORAGE_RETRY_ATTEMPTS;
        loop {
            match operation().await {
                Ok(()) => return Ok(()),
                Err(err) if remaining > 1 => {
                    let attempt = STORAGE_RETRY_ATTEMPTS - remaining + 1;
                    warn!(
                        intent = summary,
                        stage,
                        attempt,
                        error = ?err,
                        "retrying storage action"
                    );
                    remaining -= 1;
                    sleep(Duration::from_millis(STORAGE_RETRY_DELAY_MS)).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub async fn run(mut self) {
        if let Err(err) = self.load_existing_queue().await {
            warn!(error = ?err, "failed to bootstrap intent queue");
        }

        let beat_interval = self.ctx.config().beat.interval();
        let mut ticker = interval(beat_interval);
        let shutdown = self.ctx.shutdown_notifier();

        loop {
            select! {
                _ = ticker.tick() => {
                    info!("beat ticker fired");
                    self.run_beat().await;
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        OrchestratorCommand::RequestBeat => {
                            info!("beat requested by subsystem");
                            self.run_beat().await;
                        }
                    }
                }
                _ = shutdown.notified() => {
                    info!("beat orchestrator shutting down");
                    break;
                }
            }
        }
    }

    async fn run_beat(&self) {
        if let Err(err) = self.ingest_inbox() {
            warn!(error = ?err, "failed to ingest inbox");
        }

        let mut attempts: HashMap<Uuid, u8> = HashMap::new();

        loop {
            let next_intent = {
                let intents = self.ctx.intents();
                let mut queue = intents.write();
                queue.pop_next()
            };

            if let Some(intent) = next_intent {
                let intent_id = intent.id;
                match self.process_intent(&intent).await {
                    Ok(()) => {
                        attempts.remove(&intent_id);
                    }
                    Err(err) => {
                        let entry = attempts.entry(intent_id).or_insert(0);
                        *entry += 1;

                        let config = self.ctx.config();
                        let data_dir = config.data_dir.clone();
                        drop(config);

                        if *entry >= INTENT_REQUEUE_ATTEMPTS {
                            warn!(
                                intent = %intent.summary,
                                attempts = *entry,
                                error = ?err,
                                "intent failed after max retries"
                            );

                            if let Some(path) = intent.storage_path.as_ref() {
                                if let Err(move_err) =
                                    storage::quarantine_failed_intent(path, &data_dir)
                                {
                                    warn!(
                                        intent = %intent.summary,
                                        error = ?move_err,
                                        "failed to move intent to failed queue"
                                    );
                                }
                            }

                            attempts.remove(&intent_id);
                        } else {
                            warn!(
                                intent = %intent.summary,
                                attempt = *entry,
                                error = ?err,
                                "intent processing failed, will retry"
                            );
                            let intents = self.ctx.intents();
                            intents.write().push_front(intent);
                        }
                    }
                }
            } else {
                info!("no intents pending for beat");
                break;
            }
        }
    }

    fn ingest_inbox(&self) -> anyhow::Result<()> {
        let config = self.ctx.config();
        let data_dir = config.data_dir.clone();
        let threshold = config.beat.intent_threshold;
        drop(config);

        let new_intents = storage::scan_inbox(&data_dir)?;
        for record in new_intents {
            if record.intent.telos_alignment >= threshold {
                let queue_path = storage::promote_to_queue(&record.path, &data_dir)?;
                let mut intent = record.intent;
                intent.storage_path = Some(queue_path);
                let intents = self.ctx.intents();
                intents.write().push(intent);
            } else {
                storage::defer_intent(&record.path, &data_dir)?;
            }
        }

        Ok(())
    }

    async fn load_existing_queue(&self) -> anyhow::Result<()> {
        let config = self.ctx.config();
        let data_dir = config.data_dir.clone();
        drop(config);

        let existing = storage::scan_queue(&data_dir)?;
        if existing.is_empty() {
            return Ok(());
        }

        let intents = self.ctx.intents();
        let mut queue = intents.write();
        for mut record in existing {
            record.intent.storage_path = Some(record.path.clone());
            queue.push(record.intent);
        }

        Ok(())
    }
}

pub fn spawn(ctx: AppContext) -> (OrchestratorHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(32);
    let orchestrator = BeatOrchestrator::new(ctx.clone(), rx);
    let handle = OrchestratorHandle { tx: tx.clone() };
    let join = tokio::spawn(async move {
        orchestrator.run().await;
        drop(tx);
    });
    (handle, join)
}
