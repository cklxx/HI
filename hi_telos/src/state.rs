use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::Notify;

use crate::{agent::AgentRuntime, config::AppConfig, tasks::IntentQueue};

#[derive(Clone)]
pub struct AppContext {
    config: Arc<AppConfig>,
    shutdown: Arc<Notify>,
    intents: Arc<RwLock<IntentQueue>>,
    agent: Arc<AgentRuntime>,
}

impl AppContext {
    pub fn new(config: AppConfig, agent: Arc<AgentRuntime>) -> Self {
        Self {
            config: Arc::new(config),
            shutdown: Arc::new(Notify::new()),
            intents: Arc::new(RwLock::new(IntentQueue::default())),
            agent,
        }
    }

    pub fn config(&self) -> Arc<AppConfig> {
        Arc::clone(&self.config)
    }

    pub fn intents(&self) -> Arc<RwLock<IntentQueue>> {
        Arc::clone(&self.intents)
    }

    pub fn shutdown_notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.shutdown)
    }

    pub fn agent(&self) -> Arc<AgentRuntime> {
        Arc::clone(&self.agent)
    }

    pub fn request_shutdown(&self) {
        self.shutdown.notify_waiters();
    }
}
