# HI-Telos OS 工作与验收计划（ReAct MVP）

## 1. 里程碑
- **ReAct MVP（当前）**：交付包含 LLM/Agent 能力的内部闭环。
- REST 接口：`/api/intents`、`/api/sp`、`/api/logs/llm`、`/api/mock/text_structure`（GET/POST/DELETE，GET 返回 `source`、`note` 与 `updated_at` 元信息）、`/api/mock/text_structure/history`（可指定 `limit` 返回最近快照）、`/api/mock/text_structure/history/{id}`（查看单条快照）以及 `POST /api/mock/text_structure/history/{id}/restore`（恢复到任意快照）、`/api/meta/acceptance`（汇总 TODO 与验收矩阵）、`/api/meta/acceptance/module/{module}`（模块级视图，支持模糊匹配）与 `/healthz`。
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
1. 使用 `cargo run -p hi_telos --bin bootstrap_fixtures -- ./tmp/hi-telos-core && export HI_APP_ROOT=$PWD/tmp/hi-telos-core` 初始化 Mock 数据（或手动拷贝 `tests/fixtures/core`），启动服务并调用 `POST /api/intents` 写入意图；响应返回 `beat_scheduled: true`。
2. 等待心跳执行，确认：
   - Inbox 清空，对应 Markdown 存在于 `data/intent/history/`。
   - `data/journals/YYYY/MM/DD.md` 追加 ReAct 轨迹与 `Final answer: ...`。
   - `data/sp/index.json` 包含 `意图 ⇒ 最终答案`，调用 `GET /api/sp` 可见最新条目。
   - 调用 `GET /api/md/tree` 能看到上述 Markdown 路径，通过 `GET /api/md/file?path=...&render=true` 获得原文与 HTML。
   - 调用 `GET /api/logs/llm?limit=5` 返回最近的 ReAct 调用记录，包含 THINK/FINAL 阶段、Prompt 与 Response。
   - 调用 `GET /api/mock/text_structure/history?limit=3` 返回最近快照（包含 `note`），确认最新一次编辑位于列表首位；必要时可配合 `since=<RFC3339>` 或 `q=`（备注/标题/正文模糊匹配）筛选历史列表。如需恢复旧版本，可选调用 `POST /api/mock/text_structure/history/{id}/restore` 并再次 `GET /api/mock/text_structure` 验证内容与备注。
3. 调用 `GET /healthz` 返回 `ok`。
4. 执行 `cargo test`，端到端流程通过。
5. 可选：`docker compose up -d` 后访问 `http://localhost:8080/healthz`，确认容器化部署返回 `ok` 并在宿主 `./data/` 中看到落盘结果。

## 4. TODO 追踪

### 4.1 已完成清单
- [x] Orchestrator 失败重试与失败隔离：Beat 在重试期间会自动切换到失败隔离队列。
- [x] 引入 LLM Stub + ReAct Agent：Stub LLM 支持 ReAct 推理并写入 Journals/SP 指标。
- [x] 扩充 storage/agent/llm 单元测试覆盖：保证 Mock 数据、LLM 调用记录及结构化文本历史的持久化能力。
- [x] 评估真实 LLM 接入与速率治理策略：提供 OpenAI 客户端样例配置与限速策略说明。
- [x] 暴露 `/api/meta/acceptance` 聚合接口：解析任务矩阵、TODO 与验证方案用于交付状态看板。
- [x] `/api/meta/acceptance` 增强聚合统计：返回模块、待办与验证步骤计数，以及整体完成状态字段，便于前端图表展示。
- [x] `/api/meta/acceptance/module/{module}` 模块过滤视图：提供大小写/模糊匹配支持，方便前端按模块渲染局部进度。

### 4.2 进行中/待定
- 当前无新增 TODO，后续需求需先更新 PRD/TechDesign。

TODO 与技术设计保持同步，详见 [TechDesign.md](../TechDesign.md)。

## 5. 验证方案概览

| 类型 | 验证内容 | 指令/方式 |
| --- | --- | --- |
| 端到端 | Inbox → ReAct → History/SP 指标闭环 | `cargo test --test e2e` |
| 存储单元测试 | 结构化文本 Mock 数据的保存、历史与恢复 | `cargo test structured_text` |
| API 集成 | `/api/mock/text_structure`（GET/POST/DELETE/历史/恢复） | `cargo test --test server` |
| 基础健康检查 | 服务可用性与容器化部署 | `curl http://localhost:8080/healthz`（或 `docker compose up -d` 后访问） |

验证前建议运行 `cargo test` 覆盖全部单元与集成测试；如需人工验证，按照第 3 节“验收步骤”依次执行。

> 注：可通过 `GET /api/meta/acceptance` 获取完整汇总，或调用 `GET /api/meta/acceptance/module/{module}` 针对单个模块拉取最新解析结果，用于前端校验或 QA 状态看板。
