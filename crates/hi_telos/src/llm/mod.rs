use std::env;

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(&self, prompt: &str) -> anyhow::Result<String>;
    fn identity(&self) -> LlmIdentity;
}

#[derive(Debug, Default)]
pub struct LocalStubClient;

#[async_trait]
impl LlmClient for LocalStubClient {
    async fn chat(&self, prompt: &str) -> anyhow::Result<String> {
        if prompt.contains("# Phase: THINK") {
            let intent = extract_value(prompt, "Intent:").unwrap_or_else(|| "intent".to_string());
            let backlog = extract_value(prompt, "Backlog:")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_default();
            let observation = format!("Remaining backlog count: {backlog}");
            let response = serde_json::json!({
                "thought": format!("Focus on intent '{intent}' using available context"),
                "action": "summarize_intent",
                "observation": observation,
            });
            Ok(response.to_string())
        } else if prompt.contains("# Phase: FINAL") {
            let intent = extract_value(prompt, "Intent:").unwrap_or_else(|| "intent".to_string());
            let persona = extract_value(prompt, "Persona:").unwrap_or_else(|| "Agent".to_string());
            let response = serde_json::json!({
                "final_answer": format!("{persona} completed the plan for '{intent}'"),
            });
            Ok(response.to_string())
        } else {
            anyhow::bail!("stub LLM only supports THINK and FINAL phases");
        }
    }

    fn identity(&self) -> LlmIdentity {
        LlmIdentity::new("local_stub", Some("local_stub".to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiClient {
    http: Client,
    model: String,
    api_key: String,
    base_url: String,
    organization: Option<String>,
}

impl OpenAiClient {
    pub fn from_env(
        api_key_env: &str,
        model: &str,
        base_url: Option<String>,
        organization: Option<String>,
    ) -> anyhow::Result<Self> {
        let api_key = env::var(api_key_env)
            .with_context(|| format!("reading OpenAI api key from {api_key_env}"))?;
        Self::new(api_key, model, base_url, organization)
    }

    pub fn new(
        api_key: String,
        model: &str,
        base_url: Option<String>,
        organization: Option<String>,
    ) -> anyhow::Result<Self> {
        let client = Client::builder().build()?;
        let normalized_base = base_url
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
            .trim_end_matches('/')
            .to_string();

        Ok(Self {
            http: client,
            model: model.to_string(),
            api_key,
            base_url: normalized_base,
            organization,
        })
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn chat(&self, prompt: &str) -> anyhow::Result<String> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut request = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&json!({
                "model": self.model,
                "temperature": 0.2,
                "response_format": {"type": "json_object"},
                "messages": [
                    {"role": "system", "content": "You are TelosOps agent executing a ReAct loop. Always answer with valid JSON."},
                    {"role": "user", "content": prompt}
                ],
            }));

        if let Some(org) = &self.organization {
            request = request.header("OpenAI-Organization", org);
        }

        let response = request
            .send()
            .await
            .with_context(|| "sending request to OpenAI")?
            .error_for_status()
            .with_context(|| "OpenAI returned an error status")?;

        let payload: serde_json::Value = response
            .json()
            .await
            .with_context(|| "parsing OpenAI response body")?;

        payload
            .get("choices")
            .and_then(|choices| choices.as_array())
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_str())
            .map(|content| content.to_string())
            .ok_or_else(|| anyhow!("missing message content in OpenAI response"))
    }

    fn identity(&self) -> LlmIdentity {
        LlmIdentity::new("openai", Some(self.model.clone()))
    }
}

fn extract_value(prompt: &str, prefix: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .map(|value| value.trim().to_string())
}

#[derive(Debug, Clone)]
pub struct LlmIdentity {
    pub provider: &'static str,
    pub model: Option<String>,
}

impl LlmIdentity {
    pub fn new(provider: &'static str, model: Option<String>) -> Self {
        Self { provider, model }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmLogEntry {
    pub run_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub phase: String,
    pub prompt: String,
    pub response: String,
    pub provider: String,
    pub model: Option<String>,
}

impl LlmLogEntry {
    pub fn new(
        run_id: Uuid,
        timestamp: DateTime<Utc>,
        phase: impl Into<String>,
        prompt: impl Into<String>,
        response: impl Into<String>,
        identity: &LlmIdentity,
    ) -> Self {
        Self {
            run_id,
            timestamp,
            phase: phase.into(),
            prompt: prompt.into(),
            response: response.into(),
            provider: identity.provider.to_string(),
            model: identity.model.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[tokio::test]
    async fn stub_returns_react_step_payload() {
        let client = LocalStubClient;
        let response = client
            .chat(
                "# Phase: THINK\nIntent: Ship MVP\nBacklog: 4\nPersona: TelosOps\nHistory:\n(none)",
            )
            .await
            .expect("stub should handle THINK phase");

        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(parsed["action"], "summarize_intent");
        assert!(parsed["thought"].as_str().unwrap().contains("Ship MVP"));
    }

    #[tokio::test]
    async fn stub_returns_final_answer_payload() {
        let client = LocalStubClient;
        let response = client
            .chat("# Phase: FINAL\nIntent: Ship MVP\nPersona: TelosOps\nHistory:\n1. Thought")
            .await
            .expect("stub should handle FINAL phase");

        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            parsed["final_answer"],
            "TelosOps completed the plan for 'Ship MVP'"
        );
    }

    #[tokio::test]
    async fn stub_rejects_unknown_phase() {
        let client = LocalStubClient;
        let err = client.chat("# Phase: PLAN").await.unwrap_err();
        assert!(
            err.to_string()
                .contains("stub LLM only supports THINK and FINAL")
        );
    }

    #[test]
    fn extract_value_reads_prefixed_line() {
        let prompt = "Intent: Build\nBacklog: 2";
        assert_eq!(extract_value(prompt, "Intent:"), Some("Build".to_string()));
        assert_eq!(extract_value(prompt, "Backlog:"), Some("2".to_string()));
        assert_eq!(extract_value(prompt, "Persona:"), None);
    }

    #[tokio::test]
    async fn openai_client_parses_response() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/chat/completions");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"choices":[{"message":{"content":"{\"final_answer\":\"ok\"}"}}]}"#);
            })
            .await;

        let client = OpenAiClient::new(
            "test-key".to_string(),
            "gpt-test",
            Some(server.base_url()),
            None,
        )
        .expect("client should build");

        let response = client
            .chat("# Phase: THINK\nIntent: Test")
            .await
            .expect("chat should parse body");
        assert_eq!(response, "{\"final_answer\":\"ok\"}");
        mock.assert_async().await;
    }

    #[test]
    fn openai_client_requires_env_key() {
        let var = "HI_TEST_OPENAI_KEY";
        unsafe {
            env::remove_var(var);
        }
        let err = OpenAiClient::from_env(var, "gpt-test", None, None).unwrap_err();
        assert!(err.to_string().contains("reading OpenAI api key"));
    }
}
