# HI-Telos OS（人志·本愿 OS）— PRD v3.0
**Slogan：Human Intent → Telos**  
**最后更新**：2025-10-19

## 0. 电梯陈述
HI-Telos OS 是一个「以终为始」的**全自动智能人系统**。系统以**心跳（Beat）**持续运转、**全天候在线**，围绕**使命（Telos）**主动感知、思考、行动，并把一切以 **Markdown** 落盘。核心遵循：
- **有心跳与大脑**：固定节奏 + 事件触发；本地模型负责日常心跳与巡航；深度问题升级到更强模型。
- **使命与朋友**：以 Telos 对齐度（I→T）赋能优先级；能在消息 App 与社交媒体（X）中主动沟通。
- **两大工具（写入/查看）**：一切写成日记；SP 只展示**次数最多**与**最近**的日记，其余需按需检索。
- **记忆与自造工具**：自动梳理记忆（多级压缩）、自建工具与代码沙箱，产出可部署网页等成果。
- **完全放弃 KV Cache**：上下文由**最新且最多的历史思考/行动/对话记录**检索拼装；多级压缩保留“可重放”证据链。

---

## 1. 目标与非目标
### 1.1 目标
- **G1 主动性**：Beat+事件驱动生成下一步；必要时主动外联与发声。
- **G2 使命对齐**：任何行动都有 I→T 对齐分；低于阈值不执行。
- **G3 全可审计**：Prompt/推理/工具/证据/消息全落 Markdown 与结构化日志，能重放。
- **G4 全自动链**：从感知→决策→执行→发布→部署网页，全链打通。
- **G5 易运维**：单体服务 + 文件仓 + 轻索引（SQLite），Docker 一键跑。

### 1.2 非目标
- 不做复杂 UI；Web 仅只读（历史消息、Markdown 浏览、LLM 日志）。
- 不做大规模爬虫与重度知识库编辑器。

---

## 2. 用户与场景
- **Owner**：设定 Telos/章程/誓约；看日志与产物；调策略与阈值。
- **Collaborator**：通过 Telegram/X 投递意图或需求；收到回执。
- **Agent**：系统人格，负责 Beat、检索、推理、执行、发声、部署。

**关键故事**
1. Telegram 发来需求 → 生成 `intent/inbox/*.md` + I→T 打分 → 下次 Beat 入队 → 执行与回执 → 证据落盘。  
2. 每日晚间**知识整理**：选用“最智能模型”做跨日聚合与多级压缩，更新记忆卡与 SP。  
3. 深度问题（高复杂度或高价值）→ 自动升级“最高模型”，记录成本与延迟。  
4. 生成一页静态网站（报告/SP/证据集）→ 沙箱构建 → 部署到本地 `/www`（或外部托管）。

---

## 3. 范围与版本
### 3.1 MVP（4–6 周）
- Beat Orchestrator（固定间隔 + 事件触发）  
- Intent Inbox（UI 表单 + Telegram Webhook，自动 I→T 打分）  
- Mission 管理（`charter.md / northstar.md / oaths.md`）  
- Journal & Evidence（心跳日记与证据指纹化）  
- Comms：Telegram（入/出站）、X（出站 + 可选 DM 入站）  
- LLM Orchestrator（本地模型巡航；高级/最高模型回退与限额；工具调用）  
- Memory：**多级压缩**（会话树/里程碑摘要/证据链引用），替代 KV Cache  
- Publisher：静态网页产出（/www）与目录索引  
- 极简 Web：`/ui/messages`、`/ui/md`、`/ui/logs`（已交付，采用复古 SSE 长链接看板）

### 3.2 v1（+4–6 周）
- Toolsmith：声明式工具与沙箱执行回执  
- 审计回放（按誓约/项目回放整条链）  
- 搜索/索引增强（SQLite FTS5）

---

## 4. 详细需求
### 4.1 心跳（Beat）
- **频率**：默认 30 分钟；里程碑临近自动加密度；事件（消息入站）即时触发。  
- **流程**：Sense → Reflect（I→T）→ Decide → Act（LLM/工具/消息/部署）→ Log（Journal+Evidence+Metrics）。

### 4.2 模型路由策略
- **巡航/Beat**：本地小模型（低成本、快）  
- **每日知识整理（夜间批处理）**：最智能模型（云）→ 输出“记忆卡 + 压缩层”  
- **深度推理/关键决策**：最高模型（云）→ 严格证据与人工确认阈值  
- **策略输入**：任务类型、复杂度估算、预算（成本/时延/调用上限）

### 4.3 记忆与上下文（无 KV Cache）
- **无限历史，分多级压缩**：
  - L0：原始对话/行动/证据（最近 N 天原文保留）  
  - L1：会话节点评述（带引用锚点）  
  - L2：主题级总结（人物/项目/概念卡）  
  - L3：季度/年度 Telos 对齐报告  
- **上下文拼装**：优先装入**最近原文** + **高频实体** + **相关证据** + **当次目标**；其余以 L1/L2 摘要替代。

### 4.4 两大工具
- **Write**：新增/更新 `journals/YYYY/MM/DD.md`、`intent/*.md`、`mem/*.md`  
- **View**：SP 展示**次数最多**与**最近**；其他通过检索/目录浏览。

### 4.5 Web Fetch & Comms
- 抓取摘要 + 指纹（URL、hash、时间、来源）→ `evidence/*.md`  
- Telegram/X 发送与回执；入站生成 `intent/inbox/*.md`。

### 4.6 沙箱与产物
- 语言：Python/Node（可扩展）；CPU/内存/网络白名单限制  
- 能够**生成网页**并发布到 `/www`；记录构建日志与快照。

### 4.7 系统 Prompt 约束
- 首屏必须展示：**Top-Used（近 7 日引用次数最多的实体/日记）**与**Most-Recent（最近 N 条记录）**；其余内容需“主动查找/检索”后再引用。

---

## 5. 非功能需求
- **SLO**：Webhook→Inbox P95 ≤ 5s；Fetch→Evidence P95 ≤ 10s；Beat→SP 更新 P95 ≤ 10s。  
- **可靠性**：重试/熔断/回退；日志完整可追溯。  
- **安全**：密钥隔离；日志脱敏；沙箱限权。  
- **隐私**：可配置只存指纹与摘要（不存原文）。

---

## 6. API（最小集）
- `GET /api/sp`、`GET /api/md/tree`、`GET /api/md/file?path=...&render=true`  
- `GET /api/messages?dir=in|out&src=telegram|x|all`  
- `GET /api/logs/llm?level=&model=&run_id=&since=`、`GET /api/logs/llm/stream`  
- `POST /api/intent`、`POST /api/fetch`、`POST /api/beat`、`POST /webhook/telegram`

---

## 7. 验收标准（AC）
- **AC-1**：Telegram 消息 ≤5s 出现在 Inbox 与 `/ui/messages`。  
- **AC-2**：每晚整理产出 L1/L2/L3；SP 实时可见 Top-Used/Most-Recent。  
- **AC-3**：深度任务走最高模型并记录成本/延迟与证据。  
- **AC-4**：沙箱能产出并发布一页静态网站到 `/www`，日志可查。  
- **AC-5**：全链条均有 Markdown 与 JSONL 日志可重放。

---

## 8. 里程碑
- **M1**：Beat/Inbox/Journal/Evidence/LLM 本地巡航/极简 UI  
- **M2**：模型路由与回退/夜间整理/Top-Used 统计与 SP  
- **M3**：沙箱 + Publisher / X 适配 / 审计回放雏形
