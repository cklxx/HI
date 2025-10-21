use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use uuid::Uuid;

use crate::{orchestrator::OrchestratorHandle, state::AppContext, storage};

#[derive(Clone)]
pub struct ServerState {
    ctx: AppContext,
    orchestrator: OrchestratorHandle,
}

impl ServerState {
    pub fn new(ctx: AppContext, orchestrator: OrchestratorHandle) -> Self {
        Self { ctx, orchestrator }
    }

    fn ctx(&self) -> &AppContext {
        &self.ctx
    }

    fn orchestrator(&self) -> &OrchestratorHandle {
        &self.orchestrator
    }
}

pub async fn serve(state: ServerState) -> anyhow::Result<()> {
    let app = router(state.clone());
    let addr: SocketAddr = state.ctx().config().server.addr().parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state.ctx().clone()))
        .await?;

    Ok(())
}

fn router(state: ServerState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/sp", get(sp_summary))
        .route("/api/md/tree", get(md_tree))
        .route("/api/md/file", get(md_file))
        .route("/api/logs/llm", get(llm_logs))
        .route("/api/intents", post(create_intent))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn shutdown_signal(ctx: AppContext) {
    ctx.shutdown_notifier().notified().await;
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Serialize)]
struct SpSummary {
    top_used: Vec<String>,
    most_recent: Vec<String>,
}

async fn sp_summary(State(state): State<ServerState>) -> Json<SpSummary> {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    let payload = match storage::load_sp_index(&data_dir).await {
        Ok(index) => SpSummary {
            top_used: index.top_used,
            most_recent: index.most_recent,
        },
        Err(err) => {
            warn!(error = ?err, "failed to load SP index");
            SpSummary {
                top_used: vec![],
                most_recent: vec![],
            }
        }
    };

    Json(payload)
}

#[derive(Debug, Serialize)]
struct MdTreeResponse {
    files: Vec<String>,
}

async fn md_tree(State(state): State<ServerState>) -> Json<MdTreeResponse> {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    let files = match storage::list_markdown_tree(&data_dir) {
        Ok(files) => files,
        Err(err) => {
            warn!(error = ?err, "failed to list markdown tree");
            Vec::new()
        }
    };

    Json(MdTreeResponse { files })
}

#[derive(Debug, Deserialize)]
struct MdFileQuery {
    path: String,
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Serialize)]
struct MdFileResponse {
    path: String,
    content: String,
}

async fn md_file(
    State(state): State<ServerState>,
    Query(params): Query<MdFileQuery>,
) -> impl IntoResponse {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    let sanitized = match storage::sanitize_data_relative_path(&params.path) {
        Ok(path) => path,
        Err(err) => {
            warn!(error = ?err, "invalid markdown path requested");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    match storage::read_markdown_file(&data_dir, &sanitized).await {
        Ok(content) => {
            if params.render.unwrap_or(false) {
                let html = render_markdown(&content);
                Html(html).into_response()
            } else {
                Json(MdFileResponse {
                    path: sanitized.to_string_lossy().to_string(),
                    content,
                })
                .into_response()
            }
        }
        Err(err) => {
            let status = if err
                .downcast_ref::<std::io::Error>()
                .map(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
                .unwrap_or(false)
            {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            warn!(error = ?err, path = %params.path, "failed to load markdown file");
            status.into_response()
        }
    }
}

fn render_markdown(markdown: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(markdown, options);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    output
}

#[derive(Debug, Deserialize)]
struct LlmLogsQuery {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    run_id: Option<Uuid>,
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct LlmLogsResponse {
    entries: Vec<crate::llm::LlmLogEntry>,
}

async fn llm_logs(
    State(state): State<ServerState>,
    Query(params): Query<LlmLogsQuery>,
) -> impl IntoResponse {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    let since = params
        .since
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let query = storage::LlmLogQuery {
        phase: params.level.clone(),
        model: params.model.clone(),
        run_id: params.run_id,
        since,
        limit: params.limit.unwrap_or(100),
    };

    match storage::read_llm_logs(&data_dir, query).await {
        Ok(entries) => Json(LlmLogsResponse { entries }).into_response(),
        Err(err) => {
            warn!(error = ?err, "failed to read llm logs");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct NewIntentRequest {
    #[serde(default = "default_source")]
    source: String,
    summary: String,
    #[serde(default = "default_alignment")]
    telos_alignment: f32,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Serialize)]
struct NewIntentResponse {
    id: Uuid,
    path: String,
    beat_scheduled: bool,
}

async fn create_intent(
    State(state): State<ServerState>,
    Json(payload): Json<NewIntentRequest>,
) -> impl IntoResponse {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    let NewIntentRequest {
        source,
        summary,
        telos_alignment,
        body,
    } = payload;

    let persist_result =
        storage::persist_intent(&data_dir, &source, &summary, telos_alignment, &body).await;

    match persist_result {
        Ok(record) => {
            let beat_scheduled = match state.orchestrator().request_beat().await {
                Ok(()) => true,
                Err(err) => {
                    warn!(error = ?err, "failed to schedule beat after intent creation");
                    false
                }
            };

            let body = Json(NewIntentResponse {
                id: record.id,
                path: record.path.to_string_lossy().to_string(),
                beat_scheduled,
            });
            (StatusCode::ACCEPTED, body).into_response()
        }
        Err(err) => {
            warn!(error = ?err, "failed to persist intent");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn default_source() -> String {
    "user".to_string()
}

fn default_alignment() -> f32 {
    0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::AgentRuntime,
        config::AppConfig,
        orchestrator,
        state::AppContext,
        storage::{self, write_markdown},
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use std::{fs, sync::Arc};
    use tempfile::TempDir;
    use tower::ServiceExt;
    use uuid::Uuid;

    #[tokio::test]
    async fn markdown_endpoints_return_tree_and_file() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();

        fs::create_dir_all(root.join("config")).expect("config dir");
        fs::write(
            root.join("config/beat.yml"),
            "interval_minutes: 10\nintent_threshold: 0.5\n",
        )
        .expect("beat config");
        fs::write(
            root.join("config/agent.yml"),
            "max_react_steps: 1\npersona: TelosOps\n",
        )
        .expect("agent config");
        fs::write(root.join("config/llm.yml"), "provider: local_stub\n").expect("llm config");

        unsafe {
            std::env::set_var("HI_APP_ROOT", root);
            std::env::set_var("HI_SERVER_BIND", "127.0.0.1:0");
        }

        let config = AppConfig::load().expect("load config");
        let agent = AgentRuntime::from_app_config(&config).expect("agent runtime");
        let data_dir = config.data_dir.clone();
        let ctx = AppContext::new(config, Arc::new(agent));

        let (handle, join) = orchestrator::spawn(ctx.clone());

        let sample_path = data_dir.join("journals/2025/01/01.md");
        write_markdown(&sample_path, "# Heading\nBody")
            .await
            .expect("write sample");

        let state = ServerState::new(ctx.clone(), handle);
        let app = super::router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/md/tree")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("tree response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            payload["files"]
                .as_array()
                .unwrap()
                .contains(&serde_json::Value::String(
                    "journals/2025/01/01.md".to_string()
                ))
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/md/file?path=journals/2025/01/01.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("file response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let file_payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(file_payload["path"], "journals/2025/01/01.md");
        assert!(
            file_payload["content"]
                .as_str()
                .unwrap()
                .contains("# Heading")
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/md/file?path=journals/2025/01/01.md&render=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("render response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("<h1>Heading</h1>"));

        let identity = crate::llm::LlmIdentity::new("local_stub", Some("local_stub".to_string()));
        let log_entry = crate::llm::LlmLogEntry::new(
            Uuid::new_v4(),
            chrono::Utc::now(),
            "THINK",
            "prompt",
            "response",
            &identity,
        );
        storage::append_llm_logs(&data_dir, &[log_entry.clone()])
            .await
            .expect("append log");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/logs/llm?level=think&limit=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("logs response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["entries"].as_array().unwrap().len(), 1);
        assert_eq!(payload["entries"][0]["phase"], "THINK");

        ctx.request_shutdown();
        let _ = join.await;

        unsafe {
            std::env::remove_var("HI_APP_ROOT");
            std::env::remove_var("HI_SERVER_BIND");
        }
    }
}
