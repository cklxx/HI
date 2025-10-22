use std::net::SocketAddr;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
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

mod acceptance;

use crate::{
    orchestrator::OrchestratorHandle,
    state::AppContext,
    storage::{
        self, LoadedStructuredTextPreview, StructuredContent, StructuredTextHistoryEntry,
        StructuredTextHistoryFilters,
    },
};

const DEFAULT_TEXT_STRUCTURE_HISTORY_LIMIT: usize = 10;

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
    let addr: SocketAddr = state.ctx().config().server.addr().parse()?;
    let listener = TcpListener::bind(addr).await?;
    serve_with_listener(listener, state).await
}

pub async fn serve_with_listener(listener: TcpListener, state: ServerState) -> anyhow::Result<()> {
    let addr = listener.local_addr()?;
    info!(%addr, "server listening");

    let app = router(state.clone());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(state.ctx().clone()))
        .await?;

    Ok(())
}

fn router(state: ServerState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/sp", get(sp_summary))
        .route("/api/meta/acceptance", get(acceptance_overview))
        .route(
            "/api/meta/acceptance/module/:module",
            get(acceptance_module_overview),
        )
        .route("/api/md/tree", get(md_tree))
        .route("/api/md/file", get(md_file))
        .route("/api/logs/llm", get(llm_logs))
        .route(
            "/api/mock/text_structure",
            get(text_structure_preview)
                .post(update_text_structure_preview)
                .delete(reset_text_structure_preview),
        )
        .route(
            "/api/mock/text_structure/history",
            get(text_structure_history),
        )
        .route(
            "/api/mock/text_structure/history/:id",
            get(text_structure_history_entry),
        )
        .route(
            "/api/mock/text_structure/history/:id/restore",
            post(restore_text_structure_history_entry),
        )
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

async fn acceptance_overview(State(state): State<ServerState>) -> impl IntoResponse {
    let config = state.ctx().config();
    let config_dir = config.config_dir.clone();
    drop(config);

    let Some(root) = config_dir.parent() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let doc_path = root.join("docs/work_acceptance_plan.md");

    match acceptance::load_acceptance_summary(&doc_path).await {
        Ok(summary) => Json(summary).into_response(),
        Err(err) => {
            warn!(
                error = ?err,
                path = %doc_path.display(),
                "failed to load acceptance summary"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn acceptance_module_overview(
    State(state): State<ServerState>,
    Path(module): Path<String>,
) -> impl IntoResponse {
    let config = state.ctx().config();
    let config_dir = config.config_dir.clone();
    drop(config);

    let Some(root) = config_dir.parent() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let doc_path = root.join("docs/work_acceptance_plan.md");

    match acceptance::load_module_acceptance_summary(&doc_path, &module).await {
        Ok(Some(summary)) => Json(summary).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            warn!(
                error = ?err,
                module = module,
                path = %doc_path.display(),
                "failed to load acceptance module summary"
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
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

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TextStructurePreviewSource {
    Inline,
    File,
}

#[derive(Debug, Serialize, Deserialize)]
struct TextStructurePreviewResponse {
    #[serde(flatten)]
    content: StructuredContent,
    source: TextStructurePreviewSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<DateTime<Utc>>,
}

async fn text_structure_preview(
    State(state): State<ServerState>,
) -> Json<TextStructurePreviewResponse> {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    match storage::load_structured_text_preview(&data_dir).await {
        Ok(Some(LoadedStructuredTextPreview {
            content,
            note,
            updated_at,
        })) => Json(TextStructurePreviewResponse {
            content,
            source: TextStructurePreviewSource::File,
            note,
            updated_at,
        }),
        Ok(None) => Json(TextStructurePreviewResponse {
            content: StructuredContent::mock_payload(),
            source: TextStructurePreviewSource::Inline,
            note: None,
            updated_at: None,
        }),
        Err(err) => {
            warn!(error = ?err, "failed to load structured text preview; falling back to inline mock");
            Json(TextStructurePreviewResponse {
                content: StructuredContent::mock_payload(),
                source: TextStructurePreviewSource::Inline,
                note: None,
                updated_at: None,
            })
        }
    }
}

async fn update_text_structure_preview(
    State(state): State<ServerState>,
    Json(payload): Json<TextStructurePreviewUpdate>,
) -> impl IntoResponse {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    let (content, note) = payload.into_parts();

    match storage::save_structured_text_preview(&data_dir, &content, note.as_deref()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            warn!(error = ?err, "failed to persist structured text preview");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn reset_text_structure_preview(State(state): State<ServerState>) -> impl IntoResponse {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    match storage::delete_structured_text_preview(&data_dir).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            warn!(error = ?err, "failed to delete structured text preview");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct TextStructureHistoryQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    since: Option<DateTime<Utc>>,
    #[serde(default, rename = "q")]
    query: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TextStructurePreviewUpdate {
    Content(StructuredContent),
    WithNote {
        content: StructuredContent,
        #[serde(default)]
        note: Option<String>,
    },
}

impl TextStructurePreviewUpdate {
    fn into_parts(self) -> (StructuredContent, Option<String>) {
        match self {
            TextStructurePreviewUpdate::Content(content) => (content, None),
            TextStructurePreviewUpdate::WithNote { content, note } => (content, note),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TextStructureHistoryResponse {
    entries: Vec<StructuredTextHistoryEntry>,
}

async fn text_structure_history(
    State(state): State<ServerState>,
    Query(params): Query<TextStructureHistoryQuery>,
) -> Json<TextStructureHistoryResponse> {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    let TextStructureHistoryQuery {
        limit,
        since,
        query,
    } = params;
    let limit = limit.unwrap_or(DEFAULT_TEXT_STRUCTURE_HISTORY_LIMIT);
    let filters = StructuredTextHistoryFilters {
        since,
        note_query: query,
    };
    let filters = if filters == StructuredTextHistoryFilters::default() {
        None
    } else {
        Some(filters)
    };
    let filter_ref = filters.as_ref();

    match storage::list_structured_text_history(&data_dir, limit, filter_ref).await {
        Ok(entries) => Json(TextStructureHistoryResponse { entries }),
        Err(err) => {
            warn!(error = ?err, "failed to list structured text history");
            Json(TextStructureHistoryResponse {
                entries: Vec::new(),
            })
        }
    }
}

async fn text_structure_history_entry(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    match storage::load_structured_text_history_entry(&data_dir, &id).await {
        Ok(Some(entry)) => Json(entry).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            if err.root_cause().is::<chrono::ParseError>() {
                StatusCode::BAD_REQUEST.into_response()
            } else {
                warn!(error = ?err, id = %id, "failed to load structured text history entry");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

async fn restore_text_structure_history_entry(
    State(state): State<ServerState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let config = state.ctx().config();
    let data_dir = config.data_dir.clone();
    drop(config);

    match storage::restore_structured_text_preview_from_history(&data_dir, &id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            if err.root_cause().is::<chrono::ParseError>() {
                StatusCode::BAD_REQUEST.into_response()
            } else {
                warn!(error = ?err, id = %id, "failed to restore structured text history entry");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
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
        storage::{self, StructuredContent, StructuredSection, write_markdown},
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
    async fn acceptance_overview_reflects_markdown_plan() {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();

        fs::create_dir_all(root.join("config")).expect("config dir");
        fs::create_dir_all(root.join("docs")).expect("docs dir");
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
        fs::write(
            root.join("docs/work_acceptance_plan.md"),
            "## 2. 任务矩阵\n| 模块 | 任务 | 状态 |\n| --- | --- | --- |\n| API | 汇总验收计划 | ✅ |\n\n## 4. TODO 追踪\n\n### 4.1 已完成清单\n- [x] 已完成事项\n\n### 4.2 进行中/待定\n- 待处理事项\n\n## 5. 验证方案概览\n| 类型 | 验证内容 | 指令/方式 |\n| --- | --- | --- |\n| 端到端 | 核心链路 | cargo test --test e2e |\n",
        )
        .expect("plan doc");

        unsafe {
            std::env::set_var("HI_APP_ROOT", root);
            std::env::set_var("HI_SERVER_BIND", "127.0.0.1:0");
        }

        let config = AppConfig::load().expect("load config");
        let agent = AgentRuntime::from_app_config(&config).expect("agent runtime");
        let ctx = AppContext::new(config, Arc::new(agent));

        let (handle, join) = orchestrator::spawn(ctx.clone());
        let state = ServerState::new(ctx.clone(), handle);
        let app = super::router(state.clone());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/meta/acceptance")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("acceptance response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(
            payload["source"]["doc_path"]
                .as_str()
                .unwrap()
                .ends_with("docs/work_acceptance_plan.md")
        );
        assert_eq!(payload["task_matrix"].as_array().unwrap().len(), 1);
        assert_eq!(payload["completed_todos"].as_array().unwrap().len(), 1);
        assert_eq!(payload["pending_todos"].as_array().unwrap().len(), 1);
        assert_eq!(payload["validation_plan"].as_array().unwrap().len(), 1);
        assert_eq!(payload["metrics"]["modules_total"], serde_json::json!(1));
        assert_eq!(
            payload["metrics"]["modules_completed"],
            serde_json::json!(1)
        );
        assert_eq!(payload["metrics"]["todos_completed"], serde_json::json!(1));
        assert_eq!(payload["metrics"]["todos_pending"], serde_json::json!(1));
        assert_eq!(payload["metrics"]["validation_steps"], serde_json::json!(1));
        assert_eq!(
            payload["metrics"]["overall_status"],
            serde_json::json!("in_progress")
        );
        assert_eq!(
            payload["completed_todos"][0]["label"],
            serde_json::Value::String("已完成事项".to_string())
        );
        assert_eq!(
            payload["pending_todos"][0]["label"],
            serde_json::Value::String("待处理事项".to_string())
        );
        assert_eq!(
            payload["validation_plan"][0]["command"],
            serde_json::Value::String("cargo test --test e2e".to_string())
        );
        assert_eq!(
            payload["task_matrix"][0]["module"],
            serde_json::Value::String("API".to_string())
        );
        assert_eq!(
            payload["task_matrix"][0]["status"],
            serde_json::Value::String("✅".to_string())
        );

        let module_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/meta/acceptance/module/API")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("module response");
        assert_eq!(module_response.status(), StatusCode::OK);
        let body = module_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let module_payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(module_payload["module"], "API");
        assert_eq!(module_payload["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(
            module_payload["metrics"]["tasks_total"],
            serde_json::json!(1)
        );
        assert_eq!(
            module_payload["metrics"]["overall_status"],
            serde_json::json!("complete")
        );

        let fuzzy_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/meta/acceptance/module/api")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("fuzzy module response");
        assert_eq!(fuzzy_response.status(), StatusCode::OK);

        let missing_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/meta/acceptance/module/unknown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("missing module response");
        assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);

        ctx.request_shutdown();
        let _ = join.await;

        unsafe {
            std::env::remove_var("HI_APP_ROOT");
            std::env::remove_var("HI_SERVER_BIND");
        }
    }

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

        let preview_payload = StructuredContent {
            title: "Preview Title".to_string(),
            summary: "Preview summary for front-end snapshot.".to_string(),
            sections: vec![StructuredSection {
                heading: "Section".to_string(),
                body: vec!["Line".to_string()],
                children: vec![],
            }],
        };
        tokio::fs::create_dir_all(data_dir.join("mock"))
            .await
            .expect("create mock dir");
        tokio::fs::write(
            data_dir.join("mock/text_structure.json"),
            serde_json::to_string(&preview_payload).unwrap(),
        )
        .await
        .expect("write preview");

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
        storage::append_llm_logs(&data_dir, std::slice::from_ref(&log_entry))
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

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("text structure response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["title"], preview_payload.title);
        assert_eq!(payload["summary"], preview_payload.summary);
        assert_eq!(
            payload["sections"].as_array().unwrap()[0]["heading"],
            preview_payload.sections[0].heading
        );
        assert_eq!(payload["source"], "file");
        assert!(payload["updated_at"].as_str().is_some());

        ctx.request_shutdown();
        let _ = join.await;

        unsafe {
            std::env::remove_var("HI_APP_ROOT");
            std::env::remove_var("HI_SERVER_BIND");
        }
    }

    #[tokio::test]
    async fn structured_text_preview_can_be_updated_via_post() {
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
        let state = ServerState::new(ctx.clone(), handle);
        let app = super::router(state.clone());

        let desired = StructuredContent {
            title: "Custom Title".to_string(),
            summary: "Custom summary".to_string(),
            sections: vec![StructuredSection {
                heading: "Custom heading".to_string(),
                body: vec!["Line".to_string()],
                children: vec![],
            }],
        };
        let initial_note = "Initial skeleton";

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mock/text_structure")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "content": desired.clone(),
                            "note": initial_note,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("post response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let stored = tokio::fs::read_to_string(data_dir.join("mock/text_structure.json"))
            .await
            .expect("read stored");
        let stored: serde_json::Value = serde_json::from_str(&stored).expect("parse stored");
        assert_eq!(stored["content"], serde_json::to_value(&desired).unwrap());
        assert_eq!(
            stored["note"],
            serde_json::Value::String(initial_note.to_string())
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let fetched: TextStructurePreviewResponse =
            serde_json::from_slice(&body).expect("parse fetched");
        assert_eq!(fetched.content, desired);
        assert_eq!(fetched.source, TextStructurePreviewSource::File);
        assert_eq!(fetched.note.as_deref(), Some(initial_note));
        assert!(fetched.updated_at.is_some());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure/history?limit=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("history response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let history: TextStructureHistoryResponse =
            serde_json::from_slice(&body).expect("parse history");
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].content, desired);
        assert_eq!(history.entries[0].note.as_deref(), Some(initial_note));
        let history_id = history.entries[0].id.clone();

        let history_entry_uri = format!("/api/mock/text_structure/history/{history_id}");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(history_entry_uri.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("history entry response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let entry: StructuredTextHistoryEntry =
            serde_json::from_slice(&body).expect("parse history entry");
        assert_eq!(entry.id, history_id);
        assert_eq!(entry.content, desired);
        assert_eq!(entry.note.as_deref(), Some(initial_note));

        let updated = StructuredContent {
            title: "Updated".to_string(),
            summary: "Updated summary".to_string(),
            sections: vec![StructuredSection {
                heading: "Updated heading".to_string(),
                body: vec!["Updated".to_string()],
                children: vec![],
            }],
        };
        let updated_note = "Revision 2";

        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mock/text_structure")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "content": updated.clone(),
                            "note": updated_note,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("post response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response after update");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let fetched: TextStructurePreviewResponse =
            serde_json::from_slice(&body).expect("parse fetched");
        assert_eq!(fetched.content, updated);
        assert_eq!(fetched.note.as_deref(), Some(updated_note));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure/history?limit=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("history list response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let expanded: TextStructureHistoryResponse =
            serde_json::from_slice(&body).expect("parse expanded history");
        assert!(expanded.entries.len() >= 2);

        let older = expanded.entries[1].saved_at + chrono::Duration::milliseconds(1);
        let since_uri = format!(
            "/api/mock/text_structure/history?since={}",
            older.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(since_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("history since response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let filtered_since: TextStructureHistoryResponse =
            serde_json::from_slice(&body).expect("parse since history");
        assert_eq!(filtered_since.entries.len(), 1);
        assert_eq!(
            filtered_since.entries[0].note.as_deref(),
            Some(updated_note)
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure/history?q=revision")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("history query response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let filtered_query: TextStructureHistoryResponse =
            serde_json::from_slice(&body).expect("parse query history");
        assert_eq!(filtered_query.entries.len(), 1);
        assert_eq!(
            filtered_query.entries[0].note.as_deref(),
            Some(updated_note)
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure/history?q=custom")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("history content query response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let filtered_content: TextStructureHistoryResponse =
            serde_json::from_slice(&body).expect("parse content history");
        assert_eq!(filtered_content.entries.len(), 1);
        assert!(
            filtered_content.entries[0]
                .content
                .summary
                .contains("Custom summary")
        );

        let restore_uri = format!("/api/mock/text_structure/history/{history_id}/restore");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(restore_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("restore response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response after restore");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let restored: TextStructurePreviewResponse =
            serde_json::from_slice(&body).expect("parse restored");
        assert_eq!(restored.content, desired);
        assert_eq!(restored.note.as_deref(), Some(initial_note));

        ctx.request_shutdown();
        let _ = join.await;

        unsafe {
            std::env::remove_var("HI_APP_ROOT");
            std::env::remove_var("HI_SERVER_BIND");
        }
    }

    #[tokio::test]
    async fn structured_text_preview_can_be_reset_via_delete() {
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
        let state = ServerState::new(ctx.clone(), handle);
        let app = super::router(state.clone());

        let desired = StructuredContent {
            title: "Custom Title".to_string(),
            summary: "Custom summary".to_string(),
            sections: vec![StructuredSection {
                heading: "Custom heading".to_string(),
                body: vec!["Line".to_string()],
                children: vec![],
            }],
        };

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mock/text_structure")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&desired).unwrap()))
                    .unwrap(),
            )
            .await
            .expect("post response");

        assert!(data_dir.join("mock/text_structure.json").exists());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/mock/text_structure")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("delete response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        assert!(!data_dir.join("mock/text_structure.json").exists());

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/mock/text_structure")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("get response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let fetched: TextStructurePreviewResponse =
            serde_json::from_slice(&body).expect("parse fetched");
        assert_eq!(fetched.source, TextStructurePreviewSource::Inline);
        assert!(fetched.note.is_none());
        assert!(fetched.updated_at.is_none());
        assert_eq!(
            fetched.content.title,
            StructuredContent::mock_payload().title
        );

        ctx.request_shutdown();
        let _ = join.await;

        unsafe {
            std::env::remove_var("HI_APP_ROOT");
            std::env::remove_var("HI_SERVER_BIND");
        }
    }
}
