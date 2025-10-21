# HI-Telos OS 工作与验收计划（ReAct MVP）

## 1. 里程碑
- **ReAct MVP（当前）**：交付包含 LLM/Agent 能力的内部闭环。
- REST 接口：`/api/intents`、`/api/sp`、`/healthz`。
 - REST 接口：`/api/intents`、`/api/sp`、`/api/logs/llm`、`/healthz`。
  - Beat Orchestrator：定时 ticker + 内部 `request_beat`，完成 Inbox → Queue → Agent → History。
  - Agent/LLM：使用本地 Stub 执行 ReAct，写入 Journal 与 SP 指标。
  - 存储：Markdown/JSON 落盘，维护 Journals、History、SP 指标及失败隔离队列。
  - 测试：端到端验证意图 → ReAct 输出 → 指标更新。

未来的真实 LLM、多节点或通知能力暂不纳入计划；若需推进需同步更新 PRD/TechDesign。

## 2. 任务矩阵
| 模块 | 任务 | 状态 |
| --- | --- | --- |
| 配置 | 加载 `beat.yml`/`agent.yml`/`llm.yml` 并初始化目录 | ✅ |
| Intent 存储 | Inbox/Queue/Deferred/History/Journals/SP 维护 | ✅ |
| Beat Orchestrator | 定时器、Inbox 筛选、Agent 调用、归档 | ✅ |
| Agent & LLM | Stub LLM + ReAct Runtime，实现感知与最终答案 | ✅ |
| API | `/healthz`、`/api/intents`、`/api/sp`、`/api/md/tree`、`/api/md/file` | ✅ |
| 指标 | `sp/index.json` 写入 “意图 ⇒ 最终答案” | ✅ |
| 测试 | `tests/e2e.rs` 覆盖含 ReAct 输出的闭环 | ✅ |
| 文档 | PRD、TechDesign、Prototype、README、验收计划 | ✅ |

## 3. 验收步骤
1. 启动服务并调用 `POST /api/intents` 写入意图；响应返回 `beat_scheduled: true`。
2. 等待心跳执行，确认：
   - Inbox 清空，对应 Markdown 存在于 `data/intent/history/`。
   - `data/journals/YYYY/MM/DD.md` 追加 ReAct 轨迹与 `Final answer: ...`。
   - `data/sp/index.json` 包含 `意图 ⇒ 最终答案`，调用 `GET /api/sp` 可见最新条目。
   - 调用 `GET /api/md/tree` 能看到上述 Markdown 路径，通过 `GET /api/md/file?path=...&render=true` 获得原文与 HTML。
   - 调用 `GET /api/logs/llm?limit=5` 返回最近的 ReAct 调用记录，包含 THINK/FINAL 阶段、Prompt 与 Response。
3. 调用 `GET /healthz` 返回 `ok`。
4. 执行 `cargo test --manifest-path hi_telos/Cargo.toml`，端到端流程通过。
5. 可选：`docker compose up -d` 后访问 `http://localhost:8080/healthz`，确认容器化部署返回 `ok` 并在宿主 `./data/` 中看到落盘结果。

## 4. TODO
- [x] Orchestrator 失败重试与失败隔离。
- [x] 引入 LLM Stub + ReAct Agent。
- [x] 扩充 storage/agent/llm 单元测试覆盖。
- [x] 评估真实 LLM 接入与速率治理策略（新增 OpenAI 客户端并暴露配置示例）。

TODO 与技术设计保持同步，详见 [TechDesign.md](../TechDesign.md)。
