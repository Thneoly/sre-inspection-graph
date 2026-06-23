# modules/

WASM connector / rule / handler 模块,独立 Cargo workspace。

## 构建

```bash
# 一次性 — 装 wasm 目标
rustup target add wasm32-wasip2

# build 全部
cd modules && cargo build --release --target wasm32-wasip2

# build 单个
cd modules && cargo build -p hello-world --release --target wasm32-wasip2
```

产物在 `modules/target/wasm32-wasip2/release/*.wasm`。

## 目录

```
modules/
├── Cargo.toml             # workspace
├── manifest.toml          # 引擎启动时读取的模块清单
├── sdk/                   # 共享 SDK — guest 端 WIT bindings + helper
└── connectors/
    └── hello-world/       # 占位 connector,演示 SDK 用法
```

后续按 reference/MIGRATION_STATUS.md PRD-004 段落逐项添加:
- `connectors/k8s/`
- `connectors/prometheus/`
- `connectors/jaeger/`
- `connectors/flagd/`
- `connectors/k8s-events/`

以及 `rules/`(PRD-003)与 `handlers/`(PRD-001)子树。

## 与 engine 的关系

引擎(`engine-wasm` crate)用 wasmtime 加载 `*.wasm`,通过 WIT 接口
(`specs/wit/connector.wit` 等)调用 guest 函数。SDK 把 wit-bindgen 生成的
代码包成 ergonomic Rust API。

## Phase 1 状态

- `sdk/` 占位(WIT 真生成留 Step 4 后续 PR — 需要本地 `wasm32-wasip2` 目标)
- `connectors/hello-world/` 占位(返回一条假 Fact)
- 真 wasmtime 加载 in `engine-wasm` 也是 Phase 2 工作
