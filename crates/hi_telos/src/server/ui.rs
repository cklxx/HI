use std::{
    convert::Infallible,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use axum::{
    Router,
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    response::{Html, IntoResponse},
    routing::get,
};
use chrono::Local;
use serde::Serialize;
use tokio::task;
use tokio_stream::{StreamExt, wrappers::IntervalStream};
use tracing::warn;

use crate::{
    llm::LlmLogEntry,
    storage::{
        self, IntentRecord, LlmLogQuery, MemoryEntry, MemoryLevel, MemoryQuery, MessageDirection,
        MessageLogEntry, MessageLogQuery, SpIndex,
    },
};

use super::{ServerState, acceptance};

pub fn router() -> Router<ServerState> {
    Router::new()
        .route("/ui/messages", get(ui_messages))
        .route("/ui/messages/stream", get(ui_messages_stream))
        .route("/ui/md", get(ui_markdown))
        .route("/ui/md/stream", get(ui_markdown_stream))
        .route("/ui/logs", get(ui_logs))
        .route("/ui/mock", get(ui_mock_preview))
        .route("/ui/logs/stream", get(ui_logs_stream))
}

async fn ui_messages() -> Html<String> {
    let body = String::from(
        r#"<section><h2>Inbox</h2><pre id="inbox">Loading…</pre></section>
         <section><h2>Queue</h2><pre id="queue">Loading…</pre></section>
         <section><h2>Archive</h2><pre id="history">Loading…</pre></section>
         <section><h2>Telegram Inbound</h2><pre id="telegram-in">Loading…</pre></section>
         <section><h2>Telegram Outbound</h2><pre id="telegram-out">Loading…</pre></section>"#,
    );

    let script = r#"
(function() {
  const status = document.getElementById('status');
  function updateStatus(text) {
    if (status) {
      status.textContent = text;
    }
  }

  function renderLines(id, lines) {
    const target = document.getElementById(id);
    if (!target) {
      return;
    }
    if (!lines || lines.length === 0) {
      target.textContent = '—';
      return;
    }
    target.textContent = lines.join('\n');
  }

  updateStatus('连接中 …');
  const source = new EventSource('/ui/messages/stream');
  source.onopen = function() {
    updateStatus('已连接');
  };
  source.onerror = function() {
    updateStatus('连接断开，等待重试 …');
  };
  source.onmessage = function(event) {
    updateStatus('已连接');
    try {
      const payload = JSON.parse(event.data);
      renderLines('inbox', payload.inbox || []);
      renderLines('queue', payload.queue || []);
      renderLines('history', payload.history || []);
      renderLines('telegram-in', payload.telegram_in || []);
      renderLines('telegram-out', payload.telegram_out || []);
    } catch (err) {
      updateStatus('数据解析失败');
    }
  };
})();
"#;

    render_page(
        "HI Telos — Messages",
        "消息面板",
        "/ui/messages",
        &body,
        script,
    )
}

async fn ui_markdown() -> Html<String> {
    let body = String::from(
        r#"<section><h2>Markdown Tree</h2><ul id="file-list" class="tree"><li>Loading…</li></ul></section>
         <section><h2>验收概览</h2><pre id="acceptance">Loading…</pre></section>
         <section><h2>Viewer</h2><div id="file-viewer" class="viewer"><em>选择左侧 Markdown 查看内容</em></div></section>"#,
    );

    let script = r#"
(function() {
  const status = document.getElementById('status');
  function updateStatus(text) {
    if (status) {
      status.textContent = text;
    }
  }

  function clearChildren(node) {
    while (node.firstChild) {
      node.removeChild(node.firstChild);
    }
  }

  function renderAcceptance(lines) {
    const block = document.getElementById('acceptance');
    if (!block) {
      return;
    }
    if (!lines || lines.length === 0) {
      block.textContent = '暂无数据';
      return;
    }
    block.textContent = lines.join('\n');
  }

  function renderFiles(files) {
    const list = document.getElementById('file-list');
    if (!list) {
      return;
    }
    clearChildren(list);
    if (!files || files.length === 0) {
      const item = document.createElement('li');
      item.textContent = '暂无 Markdown 文件';
      list.appendChild(item);
      return;
    }

    files.forEach(function(path) {
      const item = document.createElement('li');
      const button = document.createElement('button');
      button.textContent = path;
      button.type = 'button';
      button.onclick = function() {
        loadFile(path);
      };
      item.appendChild(button);
      list.appendChild(item);
    });
  }

  function loadFile(path) {
    const viewer = document.getElementById('file-viewer');
    if (!viewer) {
      return;
    }
    viewer.innerHTML = '<em>载入中…</em>';
    fetch('/api/md/file?path=' + encodeURIComponent(path) + '&render=true')
      .then(function(response) {
        if (!response.ok) {
          throw new Error('HTTP ' + response.status);
        }
        return response.text();
      })
      .then(function(html) {
        viewer.innerHTML = html;
      })
      .catch(function(err) {
        viewer.textContent = '读取失败：' + err;
      });
  }

  updateStatus('连接中 …');
  const source = new EventSource('/ui/md/stream');
  source.onopen = function() {
    updateStatus('已连接');
  };
  source.onerror = function() {
    updateStatus('连接断开，等待重试 …');
  };
  source.onmessage = function(event) {
    updateStatus('已连接');
    try {
      const payload = JSON.parse(event.data);
      renderFiles(payload.files || []);
      renderAcceptance(payload.acceptance || []);
    } catch (err) {
      updateStatus('数据解析失败');
    }
  };
})();
"#;

    render_page(
        "HI Telos — Markdown",
        "Markdown 面板",
        "/ui/md",
        &body,
        script,
    )
}

async fn ui_logs() -> Html<String> {
    let body = String::from(
        r#"<section><h2>LLM Logs</h2><pre id="logs">Loading…</pre></section>
         <section><h2>SP Index</h2><pre id="sp">Loading…</pre></section>
         <section><h2>Memory Rollup</h2><pre id="memory">Loading…</pre></section>"#,
    );

    let script = r#"
(function() {
  const status = document.getElementById('status');
  function updateStatus(text) {
    if (status) {
      status.textContent = text;
    }
  }

  function renderLines(id, lines) {
    const target = document.getElementById(id);
    if (!target) {
      return;
    }
    if (!lines || lines.length === 0) {
      target.textContent = '—';
      return;
    }
    target.textContent = lines.join('\n\n');
  }

  updateStatus('连接中 …');
  const source = new EventSource('/ui/logs/stream');
  source.onopen = function() {
    updateStatus('已连接');
  };
  source.onerror = function() {
    updateStatus('连接断开，等待重试 …');
  };
  source.onmessage = function(event) {
    updateStatus('已连接');
    try {
      const payload = JSON.parse(event.data);
      renderLines('logs', payload.logs || []);
      renderLines('sp', payload.sp || []);
      renderLines('memory', payload.memory || []);
    } catch (err) {
      updateStatus('数据解析失败');
    }
  };
})();
"#;

    render_page("HI Telos — Logs", "日志面板", "/ui/logs", &body, script)
}

async fn ui_mock_preview() -> Html<String> {
    let body = String::from(
        r#"<section class=\"span-2\">
             <div class=\"summary-card\">
               <div class=\"summary-header\">
                 <span class=\"badge\" id=\"mock-source\">加载中…</span>
                 <h2 id=\"mock-title\">Telos Structured Preview</h2>
                 <p id=\"mock-summary\" class=\"summary-text\">加载中…</p>
               </div>
               <div class=\"meta-grid\">
                 <div>最近更新：<span id=\"mock-updated\">—</span></div>
                 <div>备注：<span id=\"mock-note\">—</span></div>
               </div>
               <div class=\"actions\">
                 <button type=\"button\" id=\"mock-refresh\" class=\"btn\">手动刷新</button>
                 <button type=\"button\" id=\"mock-seed\" class=\"btn\">自动生成示例</button>
                 <span class=\"hint-inline\">每 15 秒自动刷新</span>
               </div>
             </div>
           </section>
           <section class=\"span-2\">
             <h2>层级内容</h2>
             <ul id=\"mock-sections\" class=\"section-tree\">
               <li>加载中…</li>
             </ul>
           </section>
           <section>
             <h2>历史版本（最近 5 条）</h2>
             <ul id=\"mock-history\" class=\"history-list\">
               <li>加载中…</li>
             </ul>
           </section>
           <section>
             <h2>调试提示</h2>
             <pre class=\"hint\">使用 POST /api/mock/text_structure 推送结构化内容，或 DELETE 恢复默认内置 Mock。</pre>
           </section>"#,
    );

    let script = r#"
(function() {
  const statusEl = document.getElementById('status');
  const titleEl = document.getElementById('mock-title');
  const summaryEl = document.getElementById('mock-summary');
  const sourceEl = document.getElementById('mock-source');
  const noteEl = document.getElementById('mock-note');
  const updatedEl = document.getElementById('mock-updated');
  const sectionsEl = document.getElementById('mock-sections');
  const historyEl = document.getElementById('mock-history');
  const refreshBtn = document.getElementById('mock-refresh');
  const seedBtn = document.getElementById('mock-seed');
  const params = new URLSearchParams(window.location.search);
  const shouldAutoSeed = params.get('autoSeed') === '1';
  const REFRESH_INTERVAL = 15000;
  let autoSeedTriggered = false;
  let timerId = null;

  function setStatus(text) {
    if (statusEl) {
      statusEl.textContent = text;
    }
  }

  function setText(el, text) {
    if (el) {
      el.textContent = text;
    }
  }

  function clearChildren(node) {
    if (!node) {
      return;
    }
    while (node.firstChild) {
      node.removeChild(node.firstChild);
    }
  }

  function formatDate(value) {
    if (!value) {
      return '—';
    }
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }
    return date.toLocaleString('zh-CN', { hour12: false });
  }

  function mapSource(source) {
    if (source === 'file') {
      return '落盘文件（data/mock/text_structure.json）';
    }
    return '内置 Mock（Rust 默认）';
  }

  function createSectionNode(section) {
    const li = document.createElement('li');
    li.className = 'section-item';

    const heading = document.createElement('div');
    heading.className = 'section-heading';
    heading.textContent = section && section.heading ? section.heading : '未命名';
    li.appendChild(heading);

    if (section && Array.isArray(section.body) && section.body.length > 0) {
      const body = document.createElement('div');
      body.className = 'section-body';
      section.body.forEach(function(line) {
        const p = document.createElement('p');
        p.textContent = line;
        body.appendChild(p);
      });
      li.appendChild(body);
    }

    if (section && Array.isArray(section.children) && section.children.length > 0) {
      const nested = document.createElement('ul');
      nested.className = 'section-tree nested';
      section.children.forEach(function(child) {
        nested.appendChild(createSectionNode(child));
      });
      li.appendChild(nested);
    }

    return li;
  }

  function renderSections(sections) {
    if (!sectionsEl) {
      return;
    }
    clearChildren(sectionsEl);
    if (!Array.isArray(sections) || sections.length === 0) {
      const item = document.createElement('li');
      item.textContent = '暂无结构化内容';
      sectionsEl.appendChild(item);
      return;
    }
    sections.forEach(function(section) {
      sectionsEl.appendChild(createSectionNode(section));
    });
  }

  function renderHistory(entries) {
    if (!historyEl) {
      return;
    }
    clearChildren(historyEl);
    if (!Array.isArray(entries) || entries.length === 0) {
      const item = document.createElement('li');
      item.textContent = '暂无历史记录';
      historyEl.appendChild(item);
      return;
    }
    entries.forEach(function(entry) {
      const item = document.createElement('li');
      item.className = 'history-item';

      const heading = document.createElement('div');
      heading.className = 'history-heading';
      const label = entry.note && entry.note.trim().length > 0 ? entry.note : '未备注';
      heading.textContent = formatDate(entry.saved_at) + ' · ' + label;
      item.appendChild(heading);

      const detail = document.createElement('div');
      detail.className = 'history-detail';
      if (entry.content && entry.content.title) {
        detail.textContent = entry.content.title;
      } else if (entry.id) {
        detail.textContent = entry.id;
      } else {
        detail.textContent = '—';
      }
      item.appendChild(detail);

      historyEl.appendChild(item);
    });
  }

  function renderPreview(preview) {
    setText(titleEl, preview.title || '结构化 Mock');
    setText(summaryEl, preview.summary || '—');
    setText(sourceEl, mapSource(preview.source));
    setText(noteEl, preview.note || '—');
    setText(updatedEl, formatDate(preview.updated_at));
    renderSections(preview.sections || []);
  }

  function toggleRefresh(disabled) {
    if (!refreshBtn) {
      return;
    }
    refreshBtn.disabled = disabled;
    if (disabled) {
      refreshBtn.classList.add('disabled');
    } else {
      refreshBtn.classList.remove('disabled');
    }
  }

  function toggleSeed(disabled) {
    if (!seedBtn) {
      return;
    }
    seedBtn.disabled = disabled;
    if (disabled) {
      seedBtn.classList.add('disabled');
    } else {
      seedBtn.classList.remove('disabled');
    }
  }

  function maybeTriggerAutoSeed(preview) {
    if (!shouldAutoSeed || autoSeedTriggered) {
      return;
    }
    if (preview && preview.source === 'inline') {
      autoSeedTriggered = true;
      runSeed('自动注入（UI 自动触发）');
    } else {
      autoSeedTriggered = true;
    }
  }

  function fetchData() {
    toggleRefresh(true);
    setStatus('加载中 …');
    return Promise.all([
      fetch('/api/mock/text_structure').then(function(response) {
        if (!response.ok) {
          throw new Error('preview HTTP ' + response.status);
        }
        return response.json();
      }),
      fetch('/api/mock/text_structure/history?limit=5').then(function(response) {
        if (!response.ok) {
          throw new Error('history HTTP ' + response.status);
        }
        return response.json();
      })
    ])
      .then(function(results) {
        const preview = results[0];
        const history = results[1];
        renderPreview(preview);
        renderHistory((history && history.entries) || history || []);
        setStatus('已加载（15 秒自动刷新）');
        maybeTriggerAutoSeed(preview);
        return preview;
      })
      .catch(function(err) {
        console.error('加载 Mock 数据失败', err);
        setStatus('加载失败：' + err.message);
      })
      .finally(function() {
        toggleRefresh(false);
      });
  }

  function runSeed(noteText) {
    toggleSeed(true);
    toggleRefresh(true);
    setStatus('正在生成示例 …');
    const nowText = new Date().toLocaleString('zh-CN', { hour12: false });
    const payload = {
      note: noteText || '自动生成于 ' + nowText,
      label: 'UI Auto Mock',
      summary: 'Generated via UI mock console at ' + nowText,
    };
    return fetch('/api/mock/text_structure/seed', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(payload),
    })
      .then(function(response) {
        if (!response.ok) {
          throw new Error('seed HTTP ' + response.status);
        }
        return response.json();
      })
      .then(function() {
        return fetchData();
      })
      .catch(function(err) {
        console.error('生成示例失败', err);
        setStatus('生成示例失败：' + err.message);
        toggleRefresh(false);
      })
      .finally(function() {
        toggleSeed(false);
      });
  }

  if (refreshBtn) {
    refreshBtn.addEventListener('click', function() {
      fetchData();
    });
  }

  if (seedBtn) {
    seedBtn.addEventListener('click', function() {
      runSeed();
    });
  }

  fetchData();
  timerId = setInterval(fetchData, REFRESH_INTERVAL);

  window.addEventListener('beforeunload', function() {
    if (timerId) {
      clearInterval(timerId);
    }
  });
})();
"#;

    render_page(
        "HI Telos — Mock Preview",
        "结构化 Mock 预览",
        "/ui/mock",
        &body,
        script,
    )
}

async fn ui_messages_stream(State(state): State<ServerState>) -> impl IntoResponse {
    let mut interval = tokio::time::interval(Duration::from_secs(3));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let stream = IntervalStream::new(interval)
        .map(move |_| state.clone())
        .then(|state| async move { to_event(build_messages_payload(&state).await, "messages") });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text(": keep-alive"),
        )
        .into_response()
}

async fn ui_markdown_stream(State(state): State<ServerState>) -> impl IntoResponse {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let stream = IntervalStream::new(interval)
        .map(move |_| state.clone())
        .then(|state| async move { to_event(build_markdown_payload(&state).await, "markdown") });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text(": keep-alive"),
        )
        .into_response()
}

async fn ui_logs_stream(State(state): State<ServerState>) -> impl IntoResponse {
    let mut interval = tokio::time::interval(Duration::from_secs(4));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let stream = IntervalStream::new(interval)
        .map(move |_| state.clone())
        .then(|state| async move { to_event(build_logs_payload(&state).await, "logs") });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text(": keep-alive"),
        )
        .into_response()
}

fn render_page(
    title: &str,
    heading: &str,
    current: &str,
    body: &str,
    script: &str,
) -> Html<String> {
    let nav = format!(
        "{} | {} | {} | {}",
        nav_link("/ui/messages", current, "Messages"),
        nav_link("/ui/md", current, "Markdown"),
        nav_link("/ui/logs", current, "Logs"),
        nav_link("/ui/mock", current, "Mock Preview"),
    );

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8" />
<title>{title}</title>
<style>
body {{
  font-family: 'Courier New', monospace;
  background: #101010;
  color: #00ff90;
  margin: 0;
}}
a {{
  color: #00d0ff;
  text-decoration: none;
}}
a.active {{
  text-decoration: underline;
}}
header {{
  border-bottom: 1px solid #00ff90;
  padding: 1rem;
}}
header h1 {{
  margin: 0 0 0.5rem 0;
}}
header p {{
  margin: 0;
}}
main {{
  padding: 1rem;
  display: grid;
  gap: 1rem;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
}}
section {{
  border: 1px solid #00ff90;
  padding: 1rem;
  background: #050505;
}}
section.span-2 {{
  grid-column: span 2;
}}
@media (max-width: 720px) {{
  section.span-2 {{
    grid-column: span 1;
  }}
}}
pre {{
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
}}
ul.tree {{
  list-style: none;
  padding: 0;
  margin: 0;
}}
ul.tree li {{
  margin: 0.25rem 0;
}}
ul.tree button {{
  font-family: 'Courier New', monospace;
  background: #050505;
  color: #00ff90;
  border: 1px solid #00ff90;
  padding: 0.25rem 0.5rem;
  cursor: pointer;
}}
ul.tree button:hover {{
  background: #00ff90;
  color: #050505;
}}
.viewer {{
  min-height: 240px;
  border: 1px dashed #00ff90;
  padding: 0.5rem;
  background: #000;
  color: #e0ffe0;
}}
.summary-card {{
  display: flex;
  flex-direction: column;
  gap: 1rem;
}}
.summary-header {{
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}}
.summary-header h2 {{
  margin: 0;
}}
.summary-text {{
  margin: 0;
}}
.meta-grid {{
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 0.5rem 1rem;
  font-size: 0.9rem;
  color: #8bffd4;
}}
.badge {{
  display: inline-block;
  padding: 0.25rem 0.5rem;
  border: 1px solid #00ff90;
  border-radius: 999px;
  font-size: 0.8rem;
  letter-spacing: 0.05em;
}}
.actions {{
  display: flex;
  align-items: center;
  gap: 0.75rem;
}}
.btn {{
  background: #00ff90;
  color: #050505;
  border: none;
  padding: 0.35rem 0.75rem;
  font-family: 'Courier New', monospace;
  cursor: pointer;
}}
.btn.disabled,
.btn:disabled {{
  background: #033a2c;
  color: #7cffd1;
  cursor: not-allowed;
}}
.hint-inline {{
  font-size: 0.8rem;
  color: #8bffd4;
}}
.section-tree {{
  list-style: none;
  padding-left: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}}
.section-tree.nested {{
  margin-top: 0.5rem;
  padding-left: 1rem;
  border-left: 1px dashed #00ff90;
}}
.section-item {{
  border: 1px solid #00ff90;
  padding: 0.75rem;
  background: #020902;
}}
.section-heading {{
  font-weight: bold;
  margin-bottom: 0.5rem;
}}
.section-body {{
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}}
.section-body p {{
  margin: 0;
}}
.history-list {{
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}}
.history-item {{
  border: 1px solid #00ff90;
  padding: 0.75rem;
  background: #020902;
}}
.history-heading {{
  font-weight: bold;
  margin-bottom: 0.25rem;
}}
.history-detail {{
  font-size: 0.9rem;
  color: #8bffd4;
}}
.hint {{
  white-space: pre-wrap;
  word-break: break-word;
}}
</style>
</head>
<body>
<header>
  <h1>{heading}</h1>
  <nav>{nav}</nav>
  <p id="status">连接中 …</p>
</header>
<main>{body}</main>
<script>
{script}
</script>
</body>
</html>
"#,
        title = title,
        heading = heading,
        nav = nav,
        body = body,
        script = script,
    );

    Html(html)
}

fn nav_link(href: &str, current: &str, label: &str) -> String {
    if href == current {
        format!("<a href=\"{}\" class=\"active\">{}</a>", href, label)
    } else {
        format!("<a href=\"{}\">{}</a>", href, label)
    }
}

fn to_event<T>(result: anyhow::Result<T>, context: &'static str) -> Result<Event, Infallible>
where
    T: Serialize,
{
    match result {
        Ok(payload) => match serde_json::to_string(&payload) {
            Ok(json) => Ok(Event::default().data(json)),
            Err(err) => {
                warn!(error = ?err, %context, "failed to serialize UI payload");
                Ok(Event::default().data("{\"error\":\"serialization failure\"}"))
            }
        },
        Err(err) => {
            warn!(error = ?err, %context, "failed to build UI payload");
            Ok(Event::default().data("{\"error\":\"unavailable\"}"))
        }
    }
}

#[derive(Debug, Serialize)]
struct UiMessagesPayload {
    inbox: Vec<String>,
    queue: Vec<String>,
    history: Vec<String>,
    telegram_in: Vec<String>,
    telegram_out: Vec<String>,
}

#[derive(Debug, Serialize)]
struct UiMarkdownPayload {
    files: Vec<String>,
    acceptance: Vec<String>,
}

#[derive(Debug, Serialize)]
struct UiLogsPayload {
    logs: Vec<String>,
    sp: Vec<String>,
    memory: Vec<String>,
}

async fn build_messages_payload(state: &ServerState) -> anyhow::Result<UiMessagesPayload> {
    let data_dir = state.ctx().config().data_dir.clone();

    let inbox = spawn_scan(data_dir.clone(), storage::scan_inbox)
        .await?
        .into_iter()
        .rev()
        .take(12)
        .map(format_intent_line)
        .collect();

    let queue = spawn_scan(data_dir.clone(), storage::scan_queue)
        .await?
        .into_iter()
        .rev()
        .take(12)
        .map(format_intent_line)
        .collect();

    let history = spawn_scan(data_dir.clone(), storage::scan_history)
        .await?
        .into_iter()
        .rev()
        .take(20)
        .map(format_intent_line)
        .collect();

    let telegram_in = spawn_messages(
        data_dir.clone(),
        MessageLogQuery {
            source: Some("telegram".to_string()),
            direction: Some(MessageDirection::Inbound),
            limit: 12,
            ..Default::default()
        },
    )
    .await?
    .into_iter()
    .map(format_message_line)
    .collect();

    let telegram_out = spawn_messages(
        data_dir,
        MessageLogQuery {
            source: Some("telegram".to_string()),
            direction: Some(MessageDirection::Outbound),
            limit: 12,
            ..Default::default()
        },
    )
    .await?
    .into_iter()
    .map(format_message_line)
    .collect();

    Ok(UiMessagesPayload {
        inbox,
        queue,
        history,
        telegram_in,
        telegram_out,
    })
}

async fn spawn_scan<F>(data_dir: PathBuf, op: F) -> anyhow::Result<Vec<IntentRecord>>
where
    F: Fn(&Path) -> anyhow::Result<Vec<IntentRecord>> + Send + 'static,
{
    task::spawn_blocking(move || op(&data_dir))
        .await
        .context("scan intents join failure")?
}

async fn spawn_messages(
    data_dir: PathBuf,
    query: MessageLogQuery,
) -> anyhow::Result<Vec<MessageLogEntry>> {
    task::spawn_blocking(move || storage::read_messages(&data_dir, query))
        .await
        .context("scan messages join failure")?
}

fn format_intent_line(record: IntentRecord) -> String {
    let intent = record.intent;
    format!(
        "{} | {} | {:.2} | {}",
        intent.created_at.format("%Y-%m-%d %H:%M:%S"),
        intent.source,
        intent.telos_alignment,
        intent.summary,
    )
}

fn format_message_line(entry: MessageLogEntry) -> String {
    let stamp = entry
        .timestamp
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S");
    let direction = match entry.direction {
        MessageDirection::Inbound => "IN",
        MessageDirection::Outbound => "OUT",
    };
    let author = entry.author.clone().unwrap_or_else(|| entry.source.clone());
    let mut text = entry.text.replace('\n', " ");
    if text.len() > 160 {
        text.truncate(157);
        text.push('…');
    }
    format!(
        "{} [{}] {} #{} | {}",
        stamp, direction, author, entry.chat_id, text
    )
}

fn format_memory_entry(entry: MemoryEntry) -> String {
    let stamp = entry
        .updated_at
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S");
    let mut headline = entry.summary.replace('\n', " ");
    if headline.len() > 120 {
        headline.truncate(117);
        headline.push('…');
    }
    let details: Vec<String> = entry
        .details
        .iter()
        .take(3)
        .map(|detail| format!("   • {}", detail.replace('\n', " ")))
        .collect();
    let tail = if details.is_empty() {
        String::new()
    } else {
        format!("\n{}", details.join("\n"))
    };
    format!(
        "{} [{}] {}{}",
        stamp,
        match entry.level {
            MemoryLevel::L1 => "L1",
            MemoryLevel::L2 => "L2",
        },
        headline,
        tail,
    )
}

async fn build_markdown_payload(state: &ServerState) -> anyhow::Result<UiMarkdownPayload> {
    let (data_dir, config_dir) = {
        let config = state.ctx().config();
        (config.data_dir.clone(), config.config_dir.clone())
    };

    let files = task::spawn_blocking(move || storage::list_markdown_tree(&data_dir))
        .await
        .context("scan markdown join failure")??;

    let acceptance = acceptance_summary_lines(config_dir)
        .await
        .unwrap_or_default();

    Ok(UiMarkdownPayload { files, acceptance })
}

async fn acceptance_summary_lines(config_dir: PathBuf) -> Option<Vec<String>> {
    let root = config_dir.parent()?;
    let doc_path = root.join("docs/work_acceptance_plan.md");
    let summary = acceptance::load_acceptance_summary(&doc_path).await.ok()?;
    let metrics = summary.metrics;

    let status = match metrics.overall_status {
        acceptance::AcceptanceOverallStatus::Complete => "完成",
        acceptance::AcceptanceOverallStatus::InProgress => "进行中",
    };

    let mut lines = vec![
        format!(
            "模块完成度：{}/{}",
            metrics.modules_completed, metrics.modules_total
        ),
        format!(
            "待办：{} 待处理 / {} 已完成",
            metrics.todos_pending, metrics.todos_completed
        ),
        format!("验证步骤：{}", metrics.validation_steps),
        format!("整体状态：{}", status),
    ];

    if let Some(updated) = summary.source.updated_at {
        lines.push(format!("最近更新：{}", updated.format("%Y-%m-%d %H:%M:%S")));
    }

    Some(lines)
}

async fn build_logs_payload(state: &ServerState) -> anyhow::Result<UiLogsPayload> {
    let data_dir = state.ctx().config().data_dir.clone();

    let logs = storage::read_llm_logs(
        &data_dir,
        LlmLogQuery {
            limit: 20,
            ..Default::default()
        },
    )
    .await?
    .into_iter()
    .map(format_log_entry)
    .collect();

    let sp_lines = sp_summary_lines(&data_dir).await.unwrap_or_default();

    let memory_lines = task::spawn_blocking({
        let data_dir = data_dir.clone();
        move || {
            storage::read_memory_entries(
                &data_dir,
                MemoryQuery {
                    level: MemoryLevel::L2,
                    limit: 6,
                    since: None,
                    tag: None,
                },
            )
        }
    })
    .await
    .context("memory timeline join failure")??
    .into_iter()
    .map(format_memory_entry)
    .collect();

    Ok(UiLogsPayload {
        logs,
        sp: sp_lines,
        memory: memory_lines,
    })
}

fn format_log_entry(entry: LlmLogEntry) -> String {
    let mut prompt = entry.prompt.replace('\n', " ");
    if prompt.len() > 160 {
        prompt.truncate(157);
        prompt.push('…');
    }
    let mut response = entry.response.replace('\n', " ");
    if response.len() > 160 {
        response.truncate(157);
        response.push('…');
    }
    format!(
        "{} [{}] {}{}\n→ {}",
        entry.timestamp.with_timezone(&Local).format("%H:%M:%S"),
        entry.phase.to_uppercase(),
        entry.provider,
        entry
            .model
            .as_ref()
            .map(|model| format!("/{}", model))
            .unwrap_or_default(),
        response_line(prompt, response),
    )
}

fn response_line(prompt: String, response: String) -> String {
    format!(" {}", prompt) + "\n   ↳ " + &response
}

async fn sp_summary_lines(data_dir: &Path) -> Option<Vec<String>> {
    match storage::load_sp_index(data_dir).await {
        Ok(SpIndex {
            top_used,
            most_recent,
        }) => {
            let mut lines = Vec::new();
            if !top_used.is_empty() {
                lines.push("Top Used:".to_string());
                for item in top_used {
                    lines.push(format!("• {}", item));
                }
            }
            if !most_recent.is_empty() {
                if !lines.is_empty() {
                    lines.push(String::new());
                }
                lines.push("Most Recent:".to_string());
                for item in most_recent {
                    lines.push(format!("• {}", item));
                }
            }
            if lines.is_empty() {
                lines.push("SP 指标暂无数据".to_string());
            }
            Some(lines)
        }
        Err(err) => {
            warn!(error = ?err, "failed to load SP index for UI");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn retro_pages_render_expected_shell() {
        let Html(html) = ui_messages().await;
        assert!(html.contains("消息面板"));
        assert!(html.contains("/ui/messages/stream"));
        assert!(html.contains("telegram-in"));
        assert!(html.contains("telegram-out"));

        let Html(html) = ui_markdown().await;
        assert!(html.contains("Markdown 面板"));
        assert!(html.contains("/ui/md/stream"));

        let Html(html) = ui_logs().await;
        assert!(html.contains("日志面板"));
        assert!(html.contains("/ui/logs/stream"));
        assert!(html.contains("Memory Rollup"));
    }
}
