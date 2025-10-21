use std::{fmt::Write, sync::Arc};

use anyhow::Context;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    config::{AgentConfig, AppConfig, LlmProviderConfig},
    llm::{LlmClient, LlmLogEntry, LocalStubClient, OpenAiClient},
    tasks::Intent,
};

#[derive(Debug, Clone)]
pub struct AgentInput {
    pub intent: Intent,
    pub backlog_size: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentStep {
    pub thought: String,
    pub action: String,
    pub observation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FinalAnswer {
    pub final_answer: String,
}

#[derive(Debug, Clone)]
pub struct AgentOutcome {
    pub steps: Vec<AgentStep>,
    pub final_answer: String,
}

#[derive(Debug, Clone)]
pub struct AgentRun {
    pub outcome: AgentOutcome,
    pub llm_logs: Vec<LlmLogEntry>,
}

pub struct AgentRuntime {
    config: AgentConfig,
    llm: Arc<dyn LlmClient>,
}

impl AgentRuntime {
    pub fn new(config: AgentConfig, llm: Arc<dyn LlmClient>) -> Self {
        Self { config, llm }
    }

    pub fn from_app_config(config: &AppConfig) -> anyhow::Result<Self> {
        let llm_client: Arc<dyn LlmClient> = match &config.llm {
            LlmProviderConfig::LocalStub => Arc::new(LocalStubClient::default()),
            LlmProviderConfig::OpenAi {
                model,
                api_key_env,
                base_url,
                organization,
            } => Arc::new(OpenAiClient::from_env(
                api_key_env,
                model,
                base_url.clone(),
                organization.clone(),
            )?),
        };

        Ok(Self::new(config.agent.clone(), llm_client))
    }

    pub async fn run_react(&self, input: AgentInput) -> anyhow::Result<AgentRun> {
        let mut steps = Vec::new();
        let mut llm_logs = Vec::new();
        let run_id = Uuid::new_v4();
        let identity = self.llm.identity();

        let step_count = std::cmp::max(self.config.max_react_steps, 1);
        for step_index in 0..step_count {
            let history = format_history(&steps);
            let prompt = format!(
                "# Phase: THINK\nIntent: {}\nBacklog: {}\nPersona: {}\nStep: {}\nHistory:\n{}\nRespond with JSON containing thought, action, observation.",
                input.intent.summary,
                input.backlog_size,
                self.config.persona,
                step_index + 1,
                history,
            );

            let raw = self.llm.chat(&prompt).await?;
            llm_logs.push(LlmLogEntry::new(
                run_id,
                Utc::now(),
                "THINK",
                &prompt,
                &raw,
                &identity,
            ));
            let step: AgentStep = serde_json::from_str(&raw)
                .with_context(|| format!("parsing agent step response: {raw}"))?;
            steps.push(step);
        }

        let history = format_history(&steps);
        let final_prompt = format!(
            "# Phase: FINAL\nIntent: {}\nPersona: {}\nHistory:\n{}\nRespond with JSON containing final_answer.",
            input.intent.summary, self.config.persona, history,
        );

        let final_raw = self.llm.chat(&final_prompt).await?;
        llm_logs.push(LlmLogEntry::new(
            run_id,
            Utc::now(),
            "FINAL",
            &final_prompt,
            &final_raw,
            &identity,
        ));
        let final_payload = serde_json::from_str::<FinalAnswer>(&final_raw)
            .with_context(|| format!("parsing final answer: {final_raw}"))?;

        Ok(AgentRun {
            outcome: AgentOutcome {
                steps,
                final_answer: final_payload.final_answer,
            },
            llm_logs,
        })
    }
}

fn format_history(steps: &[AgentStep]) -> String {
    if steps.is_empty() {
        return "(none)".to_string();
    }

    let mut history = String::new();
    for (idx, step) in steps.iter().enumerate() {
        let _ = writeln!(
            &mut history,
            "{}. Thought: {} | Action: {} | Observation: {}",
            idx + 1,
            step.thought,
            step.action,
            step.observation
        );
    }
    history.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_intent() -> Intent {
        Intent {
            id: uuid::Uuid::new_v4(),
            source: "unit-test".to_string(),
            summary: "Draft launch plan".to_string(),
            telos_alignment: 0.8,
            created_at: Utc::now(),
            storage_path: None,
        }
    }

    #[test]
    fn history_formats_steps() {
        let steps = vec![
            AgentStep {
                thought: "Consider constraints".to_string(),
                action: "review_context".to_string(),
                observation: "Remaining backlog count: 2".to_string(),
            },
            AgentStep {
                thought: "Outline deliverables".to_string(),
                action: "summarize_intent".to_string(),
                observation: "Remaining backlog count: 1".to_string(),
            },
        ];

        let formatted = format_history(&steps);
        assert!(formatted.contains("1. Thought: Consider constraints"));
        assert!(formatted.contains("Action: summarize_intent"));
        assert!(!formatted.contains("(none)"));
    }

    #[test]
    fn history_defaults_to_none_when_empty() {
        assert_eq!(format_history(&[]), "(none)");
    }

    #[tokio::test]
    async fn react_runtime_yields_steps_and_final_answer() {
        let runtime = AgentRuntime::new(
            AgentConfig {
                max_react_steps: 2,
                persona: "TelosOps".to_string(),
            },
            Arc::new(LocalStubClient::default()),
        );

        let run = runtime
            .run_react(AgentInput {
                intent: sample_intent(),
                backlog_size: 3,
            })
            .await
            .expect("agent run should succeed");

        assert_eq!(run.outcome.steps.len(), 2);
        assert!(
            run.outcome
                .final_answer
                .contains("TelosOps completed the plan")
        );
        assert!(
            run.outcome
                .steps
                .iter()
                .all(|step| !step.thought.is_empty())
        );
        assert!(!run.llm_logs.is_empty());
        assert!(run.llm_logs.iter().any(|entry| entry.phase == "THINK"));
    }
}
