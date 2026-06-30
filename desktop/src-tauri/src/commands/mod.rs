//! Tauri commands 按领域分组,Phase 2 起填实:
//!
//! - `system` — 应用元信息 / 健康检查(Phase 1 已有)
//! - `wasm`   — engine-wasm runtime 桥接(Phase 1 — F)
//! - `topology` — 6 巡检视图查询(Phase 2+)
//! - `recovery` — PRD-001 actions / dry-run / execute / approval / rollback
//! - `change_events` — PRD-002 ChangeEvent CRUD + timeline
//! - `reports` — PRD-003 生成 / 订阅
//! - `connectors` — PRD-004 connector status / sync-now
//! - `fault_simulation` — 故障注入

pub mod system;
pub mod topology;
pub mod wasm;
