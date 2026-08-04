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
    ├── hello-world/       # 占位 connector(WIT 端到端验证,默认禁用)
    ├── k8s-mini/          # 多 connector 编排验证(默认禁用)
    ├── k8s/               # K8s API → topology Fact(经 kubectl proxy)
    ├── prometheus/        # PromQL → metric Fact(http-client capability)
    ├── jaeger/            # trace CHILD_OF → CALLS 边
    ├── k8s-events/        # K8s Events → change-fact(有状态 guest)
    ├── flagd/             # flag diff → change-fact(http-write)
    └── code-repo/         # 本地代码仓 → CodeRepo/Library + BUILDS/DEPENDS_ON(fs-read)
```

handler 模块(`handlers/` 子树,PRD-001 恢复动作):`scale-deploy` / `k8s-handler`(6 个 K8s action)。

## 与 engine 的关系

引擎(`engine-wasm` crate)用 wasmtime 加载 `*.wasm`,通过 WIT 接口
(`specs/wit/connector.wit` 等)调用 guest 函数。SDK 把 wit-bindgen 生成的
代码包成 ergonomic Rust API。

## Phase 1 状态

- `sdk/` 占位(WIT 真生成留 Step 4 后续 PR — 需要本地 `wasm32-wasip2` 目标)
- `connectors/hello-world/` 占位(返回一条假 Fact)
- 真 wasmtime 加载 in `engine-wasm` 也是 Phase 2 工作
