# HI-Telos OS — Prototype（极简只读 Web）
**范围**：仅用于**查看**历史消息、Markdown 文件与 LLM 日志；无视觉设计。

## 1. 导航
- `/ui`：文本入口
- `/ui/messages`：表格化列出消息（时间/来源/方向/摘要/状态）
- `/ui/md`：左侧目录树 + 右侧 Markdown 渲染（可切换“原文”）
- `/ui/logs`：LLM 日志（过滤 + SSE 实时）
- `/ui/sp`：文本 KPI（I→T/OIR/BCR/EVI）+ Most-Recent + Top-Used

## 2. 线框（ASCII）
```
/ui/messages
+--------------------------------------------------------------+
| time            | src     | dir | summary              | ... |
| 2025-10-19 12:..| telegram| in  | "周三演示..."        |     |
| ...                                                      ... |
+--------------------------------------------------------------+

/ui/md
+-----------+-----------------------------------------------+
| tree      | # 2025-10-19 — Beat v3                        |
| /mission  | ## Sense / Reflect / Decide / Act / Log       |
| /journals | ...                                           |
| /evidence |                                               |
+-----------+-----------------------------------------------+

/ui/logs
[filters: level model run_id since] [Follow ✓]
2025-10-19T01:23:45Z DEBUG llm.router request.sent run=r-001 model=local:small
2025-10-19T01:23:46Z INFO  llm.router fallback       run=r-001 -> openai:gpt-reason
...
```

## 3. 接口绑定
- `/api/messages` → `/ui/messages`
- `/api/md/tree` + `/api/md/file` → `/ui/md`
- `/api/logs/llm` + `/api/logs/llm/stream` → `/ui/logs`
- `/api/sp` → `/ui/sp`

## 4. 系统 Prompt 模版（内置约束）
```text
You are HI-Telos, an always-on, beat-driven agent.
Goals: Align Human Intent → Telos. All actions are logged to Markdown.

At start of every response:
1) Show Top-Used entities/diaries in the last 7 days (by frequency).
2) Show Most-Recent N records (by recency).
Only these two are shown by default. Everything else must be explicitly fetched via search/retrieval.

Constraints:
- Never rely on KV cache. Build context from the freshest and most-referenced thoughts/actions/dialogues.
- Prefer evidence-anchored citations (journal/evidence paths).
- For deep/critical tasks, escalate to highest-model policy. For routine beats, use local model.
```

## 5. 示例页面（最小 HTML 片段）
```html
<!doctype html><meta charset="utf-8">
<h1>HI-Telos /ui/sp</h1>
<pre id="sp"></pre>
<script>
fetch('/api/sp').then(r=>r.json()).then(j=>{
  document.getElementById('sp').textContent = JSON.stringify(j,null,2);
});
</script>
```
