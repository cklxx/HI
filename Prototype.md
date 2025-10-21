# HI-Telos OS — Prototype 指南（ReAct MVP）

MVP 仍以后端链路为主，无前端页面，仅通过 REST + 文件观察即可验证心跳与 Agent 行为。

## 1. REST 示例
```bash
# 创建 Intent，并触发内部 Beat
curl -X POST http://localhost:8080/api/intents \
  -H 'Content-Type: application/json' \
  -d '{
        "source": "cli",
        "summary": "整理 inbox",
        "telos_alignment": 0.8,
        "body": "- 检查任务\n- 更新状态"
      }'

# 查看 SP 指标（含最终答案）
curl http://localhost:8080/api/sp

# 浏览 Markdown 树与具体内容（可选渲染 HTML）
curl http://localhost:8080/api/md/tree
curl "http://localhost:8080/api/md/file?path=journals/2025/01/01.md"
curl "http://localhost:8080/api/md/file?path=journals/2025/01/01.md&render=true"

# 查看最近的 LLM 调用日志（可按 level/model/run_id/since 过滤）
curl "http://localhost:8080/api/logs/llm?limit=5"
```

创建成功返回：
```json
{
  "id": "<uuid>",
  "path": "data/intent/inbox/20240101T120000-<uuid>.md",
  "beat_scheduled": true
}
```

## 2. 目录与内容预期
触发一次心跳后，可在 `data/` 目录看到：
- `intent/history/*.md`：被消费并归档的原始 Markdown。
- `journals/YYYY/MM/DD.md`：包含 ReAct 轨迹与 `Final answer: <LLM 输出>`。
- `sp/index.json`：Top-Used / Most-Recent 列表项形如 `意图摘要 ⇒ 最终答案`。
- `logs/llm/YYYY/MM/DD.jsonl`：逐行保存 THINK/FINAL Prompt 与 Response，可追溯 Agent 推理。

## 3. 配置速览
原型默认使用仓库内的示例配置：
- `config/beat.yml`：心跳间隔与意图阈值。
- `config/agent.yml`：ReAct 步数与 Persona。
- `config/llm.yml`：
  - 默认 `local_stub`，便于在无外部依赖的情况下验证流程。
  - 可改为 `openai`，复制 `config/llm.openai.example.yml` 并设置 `model`、`api_key_env`（默认 `OPENAI_API_KEY`）。

如需实验不同 Persona 或步数，可直接修改对应 YAML 并重启进程。

## 4. Docker 运行提示
- 首次运行前执行 `mkdir -p data`，以便将宿主机目录挂载到容器内的 `/app/data`。
- 通过 `docker compose up -d` 启动后，API 监听 `http://localhost:8080`。
- 日志与文件落盘仍位于宿主机 `./data`，可直接观察 `intent/`、`journals/` 与 `sp/` 变化。
