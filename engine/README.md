# engine/

Rust workspace — primary build target from Phase 1.

10 crates:

| Crate | Phase | 一句话 |
|---|---|---|
| `engine-core` | 1+ | canonical Fact + Arrow Schema(所有下游只认它) |
| `engine-identity` | 2 | Identity Resolver(correlation-key 合并 + resolve/diff) |
| `engine-wasm` | 1+ | wasmtime runtime + capability injection |
| `engine-recovery` | 3 | PRD-001 恢复动作引擎(dry-run/审批/回滚/自动验证/链) |
| `engine-changes` | 3 | PRD-002 变更追踪(ChangeEvent/传播/yaml_diff/频率) |
| `engine-reports` | 4 | PRD-003 自检报告(3 模板 + 订阅调度 + SMTP) |
| `engine-storage` | 1+ | SQLite(latest 拓扑)+ Parquet(归档) |
| `engine-bindings` | 1+ | wasmtime 生成的 host bindings(从 `specs/wit`) |
| `engine-testkit` | 1+ | 测试 fixtures + contract runner |
| `engine-cli` | 4 | headless binary（tick 子命令） |

## 构建

```bash
cd engine
cargo check --all-targets       # 编译检查
cargo test                       # 跑测试
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

或在仓库根:

```bash
make engine-check
make engine-test
make engine-clippy
make engine-fmt
make engine-build                # release build
```

## Tauri 嵌入

`desktop/src-tauri/Cargo.toml` 通过 path 依赖 `engine-core` / `engine-storage` /
`engine-wasm`,不依赖 `engine-cli`(Tauri 自己是 binary)。

## headless 模式

`engine-cli` 是 headless binary(`tick` 单次 / `tick --loop --interval=N` 持续),加载 manifest 跑一次 `sync_all`,用于无 GUI 验证(GUI-less dump / 真集群 smoke)。桌面端走 Tauri 进程内 IPC,不起独立服务。

## 实现状态

v0.4.0 —— 全部业务 crate 已落实(recovery/changes/reports 三个 PRD + identity correlation-key 合并 + WASM capability 注入 + SQLite/Parquet 存储),非占位骨架。详见仓库根 `README.md` + `CLAUDE.md`。
