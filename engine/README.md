# engine/

Rust workspace — primary build target from Phase 1.

10 crates:

| Crate | Phase | 一句话 |
|---|---|---|
| `engine-core` | 1+ | Fact 总线 + canonical store(Arrow + DataFusion) |
| `engine-identity` | 2 | Identity Resolver(DataFusion SQL) |
| `engine-wasm` | 1+ | wasmtime runtime + capability injection |
| `engine-recovery` | 3 | PRD-001 复刻(port from `reference/app/recovery/`) |
| `engine-changes` | 3 | PRD-002 复刻(port from `reference/app/changes/`) |
| `engine-reports` | 4 | PRD-003 复刻(port from `reference/app/reports/`) |
| `engine-storage` | 1+ | SQLite + Parquet + 可选 Neo4j adapter |
| `engine-bindings` | 1+ | wasmtime 生成的 host bindings(从 `specs/wit`) |
| `engine-testkit` | 1+ | 测试 fixtures + contract runner |
| `engine-cli` | 4 | headless binary(REST + Arrow Flight) |

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

`engine-cli` 单独编译为 binary,提供 REST + Arrow Flight 服务,用于团队/SaaS 模式。
Tauri 桌面默认不启动。

## Phase 1 状态

所有 crate **骨架已落地**,内含 `pub fn placeholder()` 占位。具体复刻按
`reference/MIGRATION_STATUS.md` 表逐项推进。
