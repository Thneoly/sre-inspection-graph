//! k8s-mini — 第二条 connector。把 K8s namespace 列表当 topology Fact 喂回 host。
//!
//! 本期作用是验证 **multi-connector in WasmRuntime**:跟 hello-world 同样接
//! connector-world,但用 config_json 读 `namespaces` 列表 + 每个 namespace 产
//! 一条 Fact。
//!
//! 输入 `config_json`(可选):
//! ```json
//! {
//!   "cluster": "vm-cluster",
//!   "namespaces": ["default", "kube-system", "otel-demo"]
//! }
//! ```
//! `cluster` 缺省 → `"local"`,`namespaces` 缺省 → `["default"]`。
//!
//! 当前不调 K8s API —— http-client capability 在 host 端仍是 Phase 2 stub。Phase 3
//! 接 reqwest 后,可以把 namespaces 列表从 config 读改成 `GET /api/v1/namespaces`
//! 实拉,Fact schema 保持不变。
//!
//! cfg(target_arch = "wasm32") 守卫与 hello-world 同款 —— wit-bindgen 生成的 export
//! symbol 含 `:`/`@`,host 链接器不认。host 编译时此 crate 退化为空 crate。

#![allow(missing_docs)]

#[cfg(target_arch = "wasm32")]
mod bindings {
    wit_bindgen::generate!({
        world: "connector-world",
        path: "../../../specs/wit",
        generate_all,
    });
}

#[cfg(target_arch = "wasm32")]
use bindings::exports::sre::inspection::connector::{Fact, Guest, SyncError, SyncResult};

#[cfg(target_arch = "wasm32")]
struct K8sMini;

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
struct Config {
    #[serde(default = "default_cluster")]
    cluster: String,
    #[serde(default = "default_namespaces")]
    namespaces: Vec<String>,
}

#[cfg(target_arch = "wasm32")]
fn default_cluster() -> String {
    "local".to_string()
}

#[cfg(target_arch = "wasm32")]
fn default_namespaces() -> Vec<String> {
    vec!["default".to_string()]
}

#[cfg(target_arch = "wasm32")]
impl Default for Config {
    fn default() -> Self {
        Self {
            cluster: default_cluster(),
            namespaces: default_namespaces(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Guest for K8sMini {
    fn sync(config_json: String) -> Result<SyncResult, SyncError> {
        // 空 config / 解析失败 → 走默认值。
        // 显式 invalid JSON(传 "not-json")→ 返 SyncError::Config,让 host 看到
        // 配置错误。空字符串当成 "用默认配置" —— hello-world 也是这么处理的。
        let cfg: Config = if config_json.trim().is_empty() {
            Config::default()
        } else {
            serde_json::from_str(&config_json).map_err(|e| {
                SyncError::Config(format!("invalid config_json: {e}"))
            })?
        };

        let now_seconds = bindings::sre::inspection::clock::now_seconds();
        bindings::sre::inspection::logging::log(
            bindings::sre::inspection::logging::Level::Info,
            &format!(
                "k8s-mini sync: cluster={} namespaces={}",
                cfg.cluster,
                cfg.namespaces.len()
            ),
        );

        let facts: Vec<Fact> = cfg
            .namespaces
            .iter()
            .map(|ns| Fact {
                // id 格式与 PRD-004 K8sConnector 对齐:`{source}:{resource_id}:{ts}`
                // 这样 host 端聚合时不会跟 hello-world 撞 ID。
                id: format!("k8s-mini:ns:{}:{}:{now_seconds}", cfg.cluster, ns),
                kind: "topology-node".to_string(),
                source: "k8s-mini".to_string(),
                // resource_id 命名空间 schema:`ns:<cluster>:<namespace>`
                // 与 doc/02-L1-L2-type-and-instance-model.md Namespace 一致。
                resource_id: format!("ns:{}:{}", cfg.cluster, ns),
                resource_type: "Namespace".to_string(),
                timestamp: now_seconds,
                // attributes_json 留个 cluster 字段,便于下游做 partition 查询。
                attributes_json: format!(
                    r#"{{"cluster":"{}","namespace":"{}"}}"#,
                    cfg.cluster, ns
                ),
            })
            .collect();

        Ok(SyncResult {
            facts,
            errors: vec![],
            duration_ms: 0,
        })
    }

    fn health_check() -> bool {
        // 静态 connector — 没有外部依赖,永远健康。
        // Phase 3 接 K8s API 后改成 `GET /healthz` 探活。
        true
    }
}

#[cfg(target_arch = "wasm32")]
bindings::export!(K8sMini with_types_in bindings);
