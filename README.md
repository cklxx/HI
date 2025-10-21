# HI-Telos OS

> ReAct 驱动的最小心跳系统：Intent → Beat → 感知状态 → LLM/Agent ReAct → Journal/SP 指标。

## 仓库结构
```
HI/
├── PRD.md
├── PRD_MVP.md
├── TechDesign.md
├── Prototype.md
├── README.md
├── docs/
│   └── work_acceptance_plan.md
├── hi_telos/
│   ├── Cargo.toml
│   └── src/
│       └── ...
├── config/
│   ├── beat.yml
│   ├── agent.yml
│   └── llm.yml
└── data/ (运行时生成)
    ├── intent/{inbox,queue,history}
    ├── journals/
    ├── logs/llm/
    └── sp/
```

## 快速开始
1. 安装 Rust 1.80+。
2. 在仓库根目录执行 `cargo test --manifest-path hi_telos/Cargo.toml`，验证端到端流程（含 ReAct 输出）。
3. 启动服务：`cargo run --manifest-path hi_telos/Cargo.toml`（默认监听 `0.0.0.0:8080`）。
4. 根据 [Prototype.md](Prototype.md) 使用 REST API 投递意图并查看 Agent 行为。
5. 如需调整心跳或 Agent 行为，修改 `config/beat.yml` 与 `config/agent.yml`。
6. LLM 选择：默认使用 `config/llm.yml` 中的 `local_stub`，如需真实模型可参照下文配置 OpenAI。

## LLM 配置选项
- `local_stub`（默认）：无需外部依赖，生成可预测的 ReAct JSON，适合开发与测试。
- `openai`：
  1. 复制 `config/llm.openai.example.yml` 为 `config/llm.yml` 并填写模型名称（如 `gpt-4o-mini`）。
  2. 在环境变量中提供 `api_key_env` 指定的 Key（默认为 `OPENAI_API_KEY`）。
  3. 可选：通过 `base_url` 指向兼容的代理或 Azure OpenAI 终端，`organization` 写入组织 ID。
- 运行时会保持 ReAct Prompt 结构不变，只替换底层 LLM 客户端。

## Docker 一键部署
> 适用于无需本地安装 Rust 的场景，容器内默认挂载 `config/` 与 `data/`。

1. 构建镜像：`docker compose build`（首次运行会同步下载依赖并编译二进制）。
2. 启动服务：`docker compose up -d`。
3. 访问 `http://localhost:8080/healthz` 确认返回 `ok`，随后按照 [Prototype.md](Prototype.md) 投递意图。
4. 容器停止：`docker compose down`，数据仍保留在宿主机的 `./data/` 目录。

> 若只需快速体验，也可以直接运行 `docker run --rm -p 8080:8080 -v "$PWD/config:/app/config:ro" -v "$PWD/data:/app/data" hi-telos:latest`。

## 已实现能力
- `POST /api/intents`：写入 Inbox Markdown，触发一次心跳。
- `GET /api/sp`：读取 `sp/index.json`，返回带有 `意图 ⇒ 最终答案` 的 Top-Used / Most-Recent 列表。
- `GET /api/md/tree`：列出 `data/` 目录下的 Markdown 文件树（相对路径）。
- `GET /api/md/file?path=...&render=true|false`：读取指定 Markdown，默认返回原文，`render=true` 时返回渲染后的 HTML。
- `GET /api/logs/llm?level=&model=&run_id=&since=&limit=`：分页读取 LLM 调用日志，支持按阶段（THINK/FINAL）、模型、运行 ID 与时间过滤。
- `GET /healthz`：健康检查。
- 内部 Beat：
  - Inbox 筛选 → Queue。
  - 调用 Agent Runtime 感知 backlog 并执行 `max_react_steps` 次 ReAct 思考。
  - 将轨迹与最终答案写入 Journal，同时归档意图、更新 SP 指标。
  - 存储失败时自动重试，超过阈值后移动到 `intent/queue/failed`。

## 数据落盘
- `data/intent/inbox`：待筛选意图。
- `data/intent/queue`：等待执行的意图。
- `data/intent/queue/failed`：多次执行失败而被隔离的意图。
- `data/intent/inbox/deferred`：低于阈值的意图。
- `data/intent/history`：已经处理并归档的意图。
- `data/journals/YYYY/MM/DD.md`：包含 ReAct 轨迹与 `Final answer: ...`。
- `data/logs/llm/YYYY/MM/DD.jsonl`：逐行记录 ReAct LLM 调用的 Prompt/Response、阶段、模型信息。
- `data/sp/index.json`：记录 “意图 ⇒ 最终答案” 的 Top-Used / Most-Recent 指标。

## 下一步（如需扩展）
- 若接入除 OpenAI 外的 LLM 或高级工具链，需更新 PRD/TechDesign 并评估“能不做就不做”的约束。
- 当前 TODO 列表维护在 [docs/work_acceptance_plan.md](docs/work_acceptance_plan.md)。
