use std::{env, path::PathBuf, time::Duration};

use serde::Deserialize;
use tracing_subscriber::{EnvFilter, fmt};

use crate::storage;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub beat: BeatConfig,
    pub server: ServerConfig,
    pub agent: AgentConfig,
    pub llm: LlmProviderConfig,
    pub telegram: Option<TelegramConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BeatConfig {
    pub interval_minutes: u64,
    #[serde(default = "default_intent_threshold")]
    pub intent_threshold: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_agent_max_steps")]
    pub max_react_steps: usize,
    #[serde(default = "default_agent_persona")]
    pub persona: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum LlmProviderConfig {
    LocalStub,
    OpenAi {
        model: String,
        #[serde(default = "default_openai_api_key_env")]
        api_key_env: String,
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        organization: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default)]
    pub default_chat_id: Option<i64>,
    #[serde(default)]
    pub webhook_secret: Option<String>,
    #[serde(default = "default_telegram_api_base")]
    pub api_base: String,
}

impl AppConfig {
    pub fn load() -> anyhow::Result<Self> {
        let root = match env::var("HI_APP_ROOT") {
            Ok(path) => PathBuf::from(path),
            Err(_) => env::current_dir()?,
        };
        let data_dir = root.join("data");
        let config_dir = root.join("config");
        let beat: BeatConfig = storage::load_yaml(config_dir.join("beat.yml"))?;
        let agent: AgentConfig = storage::load_yaml(config_dir.join("agent.yml"))?;
        let llm: LlmProviderConfig = storage::load_yaml(config_dir.join("llm.yml"))?;
        let telegram = {
            let path = config_dir.join("telegram.yml");
            if path.exists() {
                Some(storage::load_yaml(path)?)
            } else {
                None
            }
        };

        storage::ensure_data_layout(&data_dir)?;

        Ok(Self {
            data_dir,
            config_dir,
            beat,
            agent,
            llm,
            telegram,
            server: ServerConfig {
                bind_addr: env::var("HI_SERVER_BIND")
                    .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            },
        })
    }
}

impl BeatConfig {
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_minutes * 60)
    }
}

impl ServerConfig {
    pub fn addr(&self) -> &str {
        &self.bind_addr
    }
}

fn default_intent_threshold() -> f32 {
    0.5
}

fn default_agent_max_steps() -> usize {
    1
}

fn default_agent_persona() -> String {
    "TelosOps".to_string()
}

fn default_openai_api_key_env() -> String {
    "OPENAI_API_KEY".to_string()
}

fn default_telegram_api_base() -> String {
    "https://api.telegram.org".to_string()
}

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
