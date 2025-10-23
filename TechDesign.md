# HI-Telos OS — 技术设计 v5.0

本版本支撑 [PRD_MVP.md](PRD_MVP.md) 所述的 **ReAct MVP**：在文件驱动的心跳系统上，引入 LLM 与 Agent 能力，实现“心跳 → 感知 → ReAct → 记录 → 下一次心跳”的最小闭环。

## 1. 架构概览
```
+--------------------+        +---------------------+
|  REST API (Axum)   |        |  Agent Runtime      |
|  - POST /api/intents|       |  - 感知队列状态          |
|  - GET  /api/sp     |       |  - 构造 ReAct Prompt  |
|  - GET  /healthz    |       |  - 调用 LLM (Stub)    |
+---------+----------+        |  - 生成最终回答        |
          |                   +-----------+---------+
          v                               |
+--------------------+                    v
| Beat Orchestrator  |          +--------------------+
| - 定时 ticker         | ------> | ReAct Output       |
| - 新意图触发一次 beat |          | - Journal 轨迹       |
| - Inbox -> Queue    |          | - SP 指标更新        |
| - Queue -> Agent    |          +--------------------+
+---------+----------+
          |
          v
+--------------------+
|  File Storage      |
| - data/intent/*    |
| - data/journals/*  |
| - data/sp/index.json|
+--------------------+
```

所有状态依然保存在本地文件系统，无数据库或外部服务依赖。

## 2. 关键流程
1. **投递意图**：`POST /api/intents` 写入 `intent/inbox` 并调用 `OrchestratorHandle::request_beat` 请求心跳。
2. **Beat 调度**：`BeatOrchestrator` 维护定时 ticker；收到请求或 ticker 触发时执行 `run_beat`。
3. **Inbox → Queue**：`ingest_inbox` 读取 Markdown，依据 `telos_alignment` 阈值筛选进入 Queue，否则移动到 `intent/inbox/deferred`。
4. **感知状态**：消费队列前通过读锁计算剩余 backlog，生成 `AgentInput`。
5. **ReAct 循环**：`AgentRuntime::run_react` 根据配置的 `max_react_steps` 和 persona 构造 Prompt，调用 `LlmClient`（MVP 为 `LocalStubClient` 或 OpenAI 客户端）生成 `AgentStep` 序列并产出 `final_answer`。同时记录每次调用的 Prompt/Response，生成 `LlmLogEntry`。
6. **持久化**：
   - `storage::append_llm_logs` 将 LLM 调用写入 `data/logs/llm/YYYY/MM/DD.jsonl`。
   - `storage::append_journal_entry` 将 ReAct 轨迹和最终答案写入当日日志。
   - `storage::archive_intent` 将 Markdown 移动到 `intent/history`。
   - `storage::update_sp_index` 使用 “意图 ⇒ 最终答案” 更新 `sp/index.json`。
7. **指标展示**：`GET /api/sp` 返回 Top-Used 与 Most-Recent，条目格式为 `意图摘要 ⇒ 最终答案 (计数)`。

## 3. 配置
- `config/beat.yml`
  ```yaml
  interval_minutes: 30
  intent_threshold: 0.6
  ```
  - `interval_minutes`：定时 ticker 周期。
  - `intent_threshold`：Inbox → Queue 阈值。
- `config/agent.yml`
  ```yaml
  max_react_steps: 1
  persona: TelosOps
  ```
  - `max_react_steps`：单次心跳中 ReAct 步数（至少为 1）。
  - `persona`：Prompt 中声明的 Agent 人设。
- `config/llm.yml`
  ```yaml
  provider: local_stub
  ```
  - `provider`：
    - `local_stub`：生成确定性 JSON 以便测试。
    - `openai`：需额外配置：
      ```yaml
      provider: openai
      model: gpt-4o-mini
      api_key_env: OPENAI_API_KEY # 可选，默认同名
      # base_url: https://api.openai.com/v1 # 可选
      # organization: your-org-id # 可选
      ```
    - OpenAI 客户端将 Prompt 按 ReAct 格式发送至 `/chat/completions`，并读取 JSON 响应。
- 环境变量
  - `HI_APP_ROOT`：自定义运行根目录（默认当前目录）。
  - `HI_SERVER_BIND`：HTTP 绑定地址（默认 `0.0.0.0:8080`）。

## 4. 模块职责
- **config**：加载 beat/agent/llm 配置、初始化数据目录、读取环境变量。
- **state**：封装 `Arc` 状态（配置、Agent Runtime、关闭通知、队列）。
- **llm**：定义 `LlmClient` 接口与 `LocalStubClient` 实现，后续可扩展真实提供方。
- **agent**：构造 ReAct Prompt、解析 LLM JSON 响应并生成 `AgentOutcome`。
- **storage**：提供 Markdown/JSON 读写、目录扫描、Journal/History/SP 维护，并生成带锚点的多级记忆摘要索引供 API 检索。
- **orchestrator**：驱动心跳、处理失败重试、调用 Agent 并落盘结果。
- **server**：Axum 路由层，暴露 API 并在写入意图后请求内部 Beat。
- **server**：Axum 路由层，暴露 API 并在写入意图后请求内部 Beat，同时提供 `GET /api/md/tree`、`GET /api/md/file` 浏览 `data/` 中的 Markdown（支持 HTML 渲染），以及 `GET /api/logs/llm` 返回 LLM 调用日志。

## 5. 测试策略
- `tests/e2e.rs`
  1. 在临时目录写入 beat/agent/llm 配置。
  2. 启动 orchestrator，提交意图。
  3. 请求一次 Beat，验证 History、Journal（含 ReAct 轨迹与 Final answer）以及 SP 指标。
- `server` 模块内的路由测试：构造临时数据，验证 `/api/md/tree`、`/api/md/file`（含 `render=true`）与 `/api/logs/llm`（含过滤参数）的输出格式。
- 后续可视需求补充 storage/agent/llm 的单元测试及真实 LLM 适配用例。

## 6. 运维
- 日志使用 `tracing`，默认 info 级别，可通过 `RUST_LOG` 调整。
- 关机流程：`CTRL+C` → 触发 shutdown 通知 → Server 与 Orchestrator 优雅退出。
- 容器化交付：`Dockerfile` 使用多阶段构建生成 release 二进制，`docker-compose.yml` 将宿主机的 `config/`、`data/` 挂载到 `/app`，并暴露 `8080` 端口。

## 7. TODO
- [x] 在 orchestrator 中保留失败重试/隔离机制。
- [x] 引入 Agent Runtime + LLM Stub，完成 ReAct 闭环。
- [x] 为 storage/agent/llm 扩充单元测试覆盖。
- [x] 评估真实 LLM 接入所需的鉴权、速率与日志策略（新增 OpenAI 客户端，提供 API Key/Org/Base URL 配置）。
- [x] 打通外部沟通通道（Telegram Webhook、消息推送）并补充 `/api/messages` 视图层。
- [x] 设计与实现多级记忆压缩流水线，产出可检索的 L1/L2 摘要索引。
- [x] 构建最小化 Web UI（`/ui/messages`、`/ui/md`、`/ui/logs`），采用复古配色复用 API 输出，并通过 SSE 长链接提供实时看板。

以上 TODO 同步记录在 [docs/work_acceptance_plan.md](docs/work_acceptance_plan.md)。
