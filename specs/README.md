# specs/

四层数据契约 source of truth(详见 [`doc/15-data-contract-spec.md`](../doc/15-data-contract-spec.md))。

```
specs/
├── version.toml        # ABI / 模板版本号 — 改这里前先读 §8 演进与版本
├── wit/                # 层 A:WIT 接口(WASM ↔ host)
│   ├── connector.wit   # connector 模块接口
│   ├── rule.wit        # rule 模块接口
│   └── handler.wit     # handler 模块接口
├── arrow/              # 层 D:Arrow schema 定义(Rust source)
│   ├── topology_fact.rs    # TopologyFact RecordBatch schema
│   ├── change_fact.rs      # ChangeEvent → Fact 映射
│   ├── alert_fact.rs       # AlertEvent → Fact 映射
│   └── metric_fact.rs      # MetricSnapshot → Fact 映射
├── tauri/              # 层 B:Tauri command 类型(Phase 1 暂空)
│   └── README.md       # 说明:tauri-specta 从 src-tauri/commands/ 自动生成
└── openapi/            # 层 C:REST OpenAPI(Phase 4 engine-cli 用,Phase 1 暂空)
    └── README.md
```

## 各层入口

| 层 | 文件 | 谁消费 |
|---|---|---|
| A — WIT | `specs/wit/*.wit` | `engine-bindings`(host side)+ `modules/sdk`(guest side) |
| B — Tauri | `desktop/src-tauri/src/commands/*.rs`(@source) → `desktop/src/api/generated.ts`(生成) | desktop Webview |
| C — REST/Flight | `specs/openapi/openapi.yaml` + Arrow Flight proto | `engine-cli` headless 模式 |
| D — Arrow | `specs/arrow/*.rs`(in-repo crate-shared schema) | engine-core canonical store |

## 版本规则

`version.toml` 是单一 source of truth。修改任何契约文件 → 同步 bump version.toml
对应字段 → CI 检查版本与 Cargo workspace `[workspace.metadata.contract_version]`
一致。详见 doc/15 §8。
