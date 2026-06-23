# specs/openapi/

REST + Arrow Flight 契约仅在 `engine-cli` headless 模式启用 — Tauri 桌面端
不通过 HTTP 通信(走 Tauri commands 见 specs/tauri/)。

Phase 4 起此目录会有:
- `openapi.yaml` — engine-cli REST API 规约(查询 / 健康 / 接收变更 webhook)
- `flight_dataset.proto` — Arrow Flight DoGet 端点定义

Phase 1 暂空,仅占位。
