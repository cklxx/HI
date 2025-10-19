# HI-Telos OS — 技术设计 v3.0
**目标**：心跳驱动、全天候、全自动；无 KV Cache，以检索拼装上下文；多级压缩记忆；本地模型巡航 + 高阶模型升级；带沙箱与网页发布。

## 1. 架构总览
```mermaid
flowchart LR
  subgraph Web[极简 Web UI]
    M[Messages]:::ui -->|REST| API
    D[MD Viewer]:::ui -->|REST| API
    L[LLM Logs]:::ui -->|REST/SSE| API
  end
  subgraph App[App Server / Axum]
    API[REST/SSE] --> Orchestrator[Beat Orchestrator]
    API --> Files[File Service]
    API --> Index[SQLite Index]
  end
  Orchestrator --> Scorer[I→T Scorer]
  Orchestrator --> Router[LLM Router]
  Router --> Local[(Local Model)]
  Router --> CloudFast[(Smart Model)]
  Router --> CloudMax[(Highest Model)]
  Orchestrator --> Tools[Tool Runtime]
  Tools --> Fetcher[Web Fetcher]
  Tools --> Sandbox[Code Sandbox]
  Sandbox --> Publisher[Static Publisher (/www)]
  Orchestrator --> Comms[Telegram/X Adapters]
  Files -.append.-> Store[(Markdown File Store)]
  Index <--> Store
  Logs[(JSONL Logs)] <-- all -- App
classDef ui fill:#fff,stroke:#999,stroke-width:1px;
```

## 2. 组件职责
- **App Server**：静态页 + REST + SSE；Auth（Admin/Readonly）。
- **Beat Orchestrator**：cron + 事件；执行 Sense/Reflect/Decide/Act/Log。
- **I→T Scorer**：读 `mission/*.md`、`oaths.md`，输出对齐度分数。
- **LLM Router**：按任务类型/预算/复杂度选择 `local/smart/highest`；失败回退。
- **Tool Runtime**：统一工具协议（web_fetch/post_telegram/post_x/write_journal/publish）。
- **Web Fetcher**：抓取正文、摘要、指纹（URL/hash/时间/来源）。
- **Comms**：Telegram（Webhook/Send）、X（Send+可选入站）。
- **Sandbox**：受限执行 Python/Node；构建产物；收集构建日志。
- **Publisher**：将产物发布至 `/www` 并生成索引页。
- **File Service**：Markdown 落盘与读取；CommonMark 渲染。
- **Index（SQLite）**：元数据/频次/最近项/日志指针；FTS5（v1）。
- **Logs（JSONL + OpenTelemetry）**：结构化、可过滤、SSE 推送。

## 3. 数据与目录
```
/data/
  /mission/charter.md|northstar.md|oaths.md
  /intent/{inbox|queue}/*.md
  /journals/YYYY/MM/DD.md
  /mem/*.md                    # 记忆卡（人物/项目/概念）
  /evidence/*.md
  /comms/{telegram,x}/{in|out}/*.md
  /www/                        # 发布的静态网站
  /sp/index.{json,md}
  /logs/llm/YYYY-MM-DD.jsonl
  /logs/system/YYYY-MM-DD.jsonl
/config/beat.yml
```

### 3.1 Front-matter（示例）
```yaml
# intent/inbox/2025-10-19-001.md
source: telegram@Chris
summary: 周三演示需要可跑 demo
telos_alignment: 0.72
oath_link: Oath#17
created_at: 2025-10-19T01:12:00Z
status: inbox
tags: [demo, projectZ]
```

## 4. 记忆体系（多级压缩）
- **L0 原文层**：最近 N 天原文 + 全部证据原件。
- **L1 节点层**：对话/行动的阶段性摘要，保留锚点（路径+行号）。
- **L2 主题层**：人物/项目/概念卡（mem/*.md），记录频次与最近引用。
- **L3 报告层**：周/月/季度对齐报告（自动生成）。
- **上下文构建**：
  1) 取 L0 最近原文（时间窗口）  
  2) 注入 Top-Used 实体（近 7 日频次）  
  3) 关联证据（evidence/*）与誓约项  
  4) 其余以 L1/L2 摘要代替原文  
  5) 控制长度，保留可重放链接

## 5. 模型路由与策略
```yaml
llm:
  router:
    policies:
      beat: ["local:small", "cloud:smart"]
      nightly_knowledge: ["cloud:smart"]
      deep_reasoning: ["cloud:highest", "cloud:smart"]
    budget:
      max_cost_per_run_usd: 0.50
      p95_latency_ms: 60000
    escalation:
      rules:
        - if: "task=deep_reasoning OR risk=high OR impact=high"
          then: "cloud:highest"
```
- **成本统计**：输入/输出 token、延迟、重试、费用；写入 `/logs/llm/*.jsonl`。

## 6. 系统 Prompt（内置要求）
```text
System:
You are HI-Telos, a beat-driven always-on agent.
Display first:
- Top-Used entities/diaries (last 7d, by frequency)
- Most-Recent records (by recency)
Everything else must be actively looked up via retrieval.

Never use KV cache. Build context from latest and most-referenced records.
Prefer evidence paths (journals/evidence). Escalate per policy when depth is needed.
```

## 7. 关键流程伪代码
```text
onBeat():
  intents = scan("/data/intent/inbox")
  score = I2T(intents, mission, oaths)
  queue = select(intents, score >= threshold)
  plan = decide(queue, deadlines, resources)
  for step in plan:
    run = router.execute(step)        # local/smart/highest
    logJSONL(run)
    writeJournal(step, run.summary, run.evidence)
  updateSP()
```

## 8. 日志规范（LLM JSONL）
```json
{"ts":"2025-10-19T01:23:45Z","level":"DEBUG","trace_id":"t1","run_id":"r1",
 "component":"llm.router","task":"beat","model":"local:small","event":"sent",
 "tokens_in":812,"budget_ms":60000,"payload_ref":"/data/runs/r1/input.md"}
```

## 9. 安全与运维
- 环境变量存密钥；脱敏日志；原始 payload 仅本地可读。
- /metrics 暴露 Prometheus；健康检查 `/healthz`。

## 10. 部署（Docker Compose）
- 服务：app（8080）、sandbox（隔离容器）、sqlite（文件级）。
- 挂载：`/data` 与 `/config`。

## 11. 测试
- 单元：I→T、路由、重试、Fetch 抽取。
- 集成：Webhook→Inbox→Beat→LLM→Evidence→Comms→Publisher。
- 性能：并发推理 50/100/200；日志滚动稳定性。
