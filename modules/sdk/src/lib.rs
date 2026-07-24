//! module-sdk — 共享 guest 端 SDK。
//!
//! Phase 1 仅提供 Fact / SyncResult / SyncError 等数据结构(与 WIT 接口
//! 字段对齐),以便 connector / rule / handler 在 host 端 dev build 跑测试。
//! 真实 WIT bindings 生成留 Step 4 后续 PR 接入 wit-bindgen 宏。
//!
//! 注:不用 no_std。`wasm32-wasip2` 是 Tier 2 stable rustc target,带完整
//! libstd,no_std 只在 wasm32-unknown-unknown 那种纯 core 环境下才需要。

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// 与 specs/wit/connector.wit 的 `fact` record 对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub resource_id: String,
    pub resource_type: String,
    pub timestamp: u64,
    pub attributes_json: String,
}

/// connector.sync 返回值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResult {
    pub facts_emitted: u64,
    pub errors: Vec<String>,
    pub duration_ms: u64,
}

/// connector.sync 错误枚举。与 WIT `sync-error` variant 对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "msg", rename_all = "kebab-case")]
pub enum SyncError {
    Config(String),
    Runtime(String),
    Timeout,
    CapabilityDenied(String),
}

/// 共享命名约定 —— 多个 guest connector(k8s / jaeger …)必须产出**同一套**
/// `resource_id`,否则跨 connector 的边会因端点不匹配而悬空。集中在此防漂移
/// (对照 C2 词表漂移 / Phase 5 node-impact bug 教训:同一约定两处硬编码必漂)。
pub mod naming {
    /// strip release prefix(`otel-demo-cartservice` → `cartservice`)。
    pub fn strip_release_prefix<'a>(name: &'a str, release_prefix: &str) -> &'a str {
        let p = format!("{release_prefix}-");
        name.strip_prefix(&p).unwrap_or(name)
    }

    /// 从 deployment 名推 ApplicationComponent 短名(对照 reference
    /// `normalize_component_name`):strip release prefix + 砍 `"service"` 后缀
    /// (仅当剩余长度 > `"service"`,以免 `adservice` → 空)+ 拆 3 个混淆名。
    pub fn normalize_component_name(deploy_name: &str, release_prefix: &str) -> String {
        let mut name = strip_release_prefix(deploy_name, release_prefix).to_string();
        if name.ends_with("service") && name.len() > "service".len() {
            name.truncate(name.len() - "service".len());
        }
        name = name
            .replace("frauddetection", "fraud-detection")
            .replace("productcatalog", "product-catalog")
            .replace("frontendproxy", "frontend-proxy");
        name
    }

    /// ApplicationComponent `resource_id`:`comp:{cluster}:{namespace}:{short}`。
    /// k8s connector 产 ApplicationComponent 节点用此 id;jaeger 产 CALLS 边端点
    /// 用同一 id,边才能挂上 k8s 已建的 comp 节点。
    pub fn component_id(cluster: &str, namespace: &str, short: &str) -> String {
        format!("comp:{cluster}:{namespace}:{short}")
    }

    /// 规范化容器镜像引用(Phase 8.2 C1 —— k8s 与 code-repo 产**同一** correlation key
    /// `image-ref:<norm>` 的前提,否则同镜像两源 key 不等无法合并)。
    ///
    /// 规则(最小集,otel-demo 够用):
    /// 1. 砍 digest:`img@sha256:...` -> `img`(digest-pinned 合并留 v2)
    /// 2. 剥隐式 `docker.io/` 前缀(显式 ghcr.io 等不动)
    /// 3. 末段(最后一个 `/` 后)无 `:tag` 则补 `:latest`(注意只看末段,registry
    ///    port 如 `localhost:5000/x` 不误判)
    pub fn normalize_image_ref(image: &str) -> String {
        let img = image.rsplit_once('@').map(|(base, _)| base).unwrap_or(image);
        let img = img.strip_prefix("docker.io/").unwrap_or(img);
        let last_seg = img.rsplit_once('/').map(|(_, name)| name).unwrap_or(img);
        if last_seg.contains(':') {
            img.to_string()
        } else {
            format!("{img}:latest")
        }
    }
}

pub use naming::{component_id, normalize_component_name, normalize_image_ref, strip_release_prefix};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_round_trips_json() {
        let fact = Fact {
            id: "fact-1".to_string(),
            kind: "topology-node".to_string(),
            source: "hello-world".to_string(),
            resource_id: "demo:cluster:ns:pod".to_string(),
            resource_type: "Pod".to_string(),
            timestamp: 1_700_000_000,
            attributes_json: "{\"foo\":\"bar\"}".to_string(),
        };
        let s = serde_json::to_string(&fact).unwrap();
        let back: Fact = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, "fact-1");
    }

    #[test]
    fn sync_error_round_trips() {
        let err = SyncError::Timeout;
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("timeout"));
    }

    #[test]
    fn normalize_component_name_pins_reference_edge_cases() {
        // 对照 reference k8s_mapper 规则 —— 这些是跨 connector 共享约定的契约。
        use super::naming::*;
        // strip release prefix
        assert_eq!(strip_release_prefix("otel-demo-cartservice", "otel-demo"), "cartservice");
        // 砍 "service" 仅当剩余长度 > 7(adservice→ad,cartservice→cart)
        assert_eq!(normalize_component_name("otel-demo-adservice", "otel-demo"), "ad");
        assert_eq!(normalize_component_name("otel-demo-cartservice", "otel-demo"), "cart");
        assert_eq!(normalize_component_name("otel-demo-paymentservice", "otel-demo"), "payment");
        assert_eq!(normalize_component_name("otel-demo-recommendationservice", "otel-demo"), "recommendation");
        // frontend 不带 service 后缀,原样
        assert_eq!(normalize_component_name("otel-demo-frontend", "otel-demo"), "frontend");
        // 3 个混淆名拆分
        assert_eq!(normalize_component_name("otel-demo-frauddetectionservice", "otel-demo"), "fraud-detection");
        assert_eq!(normalize_component_name("otel-demo-productcatalogservice", "otel-demo"), "product-catalog");
        assert_eq!(normalize_component_name("otel-demo-frontendservice", "otel-demo"), "frontend"); // service 砍,无 proxy 拆分
        assert_eq!(normalize_component_name("otel-demo-frontendproxy", "otel-demo"), "frontend-proxy");
        // component_id 拼装
        assert_eq!(component_id("vm-cluster", "otel-demo", "cart"), "comp:vm-cluster:otel-demo:cart");
    }

    #[test]
    fn normalize_image_ref_rules() {
        use super::naming::normalize_image_ref;
        // digest 砍
        assert_eq!(normalize_image_ref("ghcr.io/x/cart@sha256:abc"), "ghcr.io/x/cart:latest");
        // :latest 补(末段无 tag)
        assert_eq!(normalize_image_ref("ghcr.io/open-telemetry/demo/cart"), "ghcr.io/open-telemetry/demo/cart:latest");
        // 有 tag 不动
        assert_eq!(normalize_image_ref("ghcr.io/open-telemetry/demo/cart:1.0.0"), "ghcr.io/open-telemetry/demo/cart:1.0.0");
        // 隐式 docker.io 剥
        assert_eq!(normalize_image_ref("docker.io/library/redis:7"), "library/redis:7");
        // registry port 不误判(:5000 在 / 前,末段 cart 无 : -> 补 latest)
        assert_eq!(normalize_image_ref("localhost:5000/cart"), "localhost:5000/cart:latest");
        // 无 / 无 tag
        assert_eq!(normalize_image_ref("redis"), "redis:latest");
        assert_eq!(normalize_image_ref("redis:7"), "redis:7");
    }
}
