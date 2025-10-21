# HI-Telos OS — ReAct MVP PRD

## 目标
在“能不做就不做”的约束下交付一个包含 **LLM 与 Agent 能力** 的最小闭环：
意图落盘 → 心跳触发 → 感知系统状态 → 基于 ReAct 的推理与行动 → 记录日志/产出结论 → 归档并更新指标 → 等待下一次心跳。

## 用户故事
1. **提交意图**：内部用户通过 HTTP API 上传 Markdown 意图，系统将其写入 Inbox 并排队等候处理。
2. **自动调度**：收到新意图或定时器触发时，Beat Orchestrator 会启动一次心跳循环。
3. **感知状态**：心跳开始前，系统评估当前队列积压量等基础状态，作为 Agent 感知输入。
4. **ReAct 推理执行**：Agent 依赖内置 LLM（本地 Stub）进行至少一次 “思考 → 行动 → 观察” 的 ReAct 循环，并生成最终响应。
5. **日志与归档**：系统将 ReAct 轨迹与最终答案写入 Journal，同时归档意图 Markdown。
6. **指标查看**：`/api/sp` 输出最近一次处理的意图及对应的 Agent 结论，方便业务侧快速洞察。
7. **健康检测**：运维通过 `/healthz` 了解服务可用性。

## 范围
- **包含**
  - 单节点部署，所有状态持久化至 `data/`（Inbox/Queue/History/Journals/SP 指标/LLM 日志）。
  - HTTP 接口：`/api/intents`、`/api/sp`、`/api/md/tree`、`/api/md/file`、`/api/logs/llm`、`/healthz`。
  - Beat 调度支持定时 ticker 与新意图触发；处理链路内置失败重试与隔离队列。
  - ReAct Agent：
    - 感知输入包括意图摘要与当前队列长度。
    - 使用 `config/agent.yml` 控制最大 ReAct 步数与 Persona。
    - 使用 `config/llm.yml` 声明 LLM 提供方（默认 `local_stub`，可切换到 OpenAI Chat Completions）。
  - Journal 记录 ReAct 轨迹与最终答案；SP 指标展示 “意图 ⇒ 最终答案”。
- **不包含**
  - 向量数据库、工具调用扩展等高级 Agent 能力。
  - 多节点/多意图并行调度、优先级调度策略。
  - 面向外部的手动心跳 API 或 UI。
  - 超出 Persona/步数以外的 Agent 配置。

## 验收标准
1. `cargo test` 中的端到端用例通过：投递意图后心跳触发，最终在 History、Journal、SP 指标、LLM 日志中能看到对应记录。
2. Journal 当天文件包含 ReAct 轨迹以及 “Final answer: <LLM 输出>”。
3. `/api/sp` 的 `top_used` 与 `most_recent` 列表展示格式 `意图摘要 ⇒ 最终答案`，能看到最新处理结果。
4. Inbox 在处理完成后无残留文件，History 下出现归档的 Markdown。
5. `/healthz` 返回 `ok`。
6. `/api/logs/llm` 可按阶段/模型过滤，并返回包含 Prompt 与 Response 的记录。

## 发布节奏
- 单次交付 ReAct MVP，后续新增能力需同步更新 PRD 与技术设计。

## 开放问题
- 是否需要将 ReAct 轨迹以结构化格式暴露给外部消费者？当前仅落盘在 Journal。
- 当引入真实 LLM 时是否需要并发/速率控制？未来扩展再评估。
