//! Identity Resolver v1 — correlation-key merge(Phase 8.2 / doc/11 §4-5 minimal)。
//!
//! [`crate::topology::resolve`] 在委托 `engine_core::facts_to_graph` 之前调
//! [`rewrite_by_correlation_key`]:把共享任一 `correlation_keys` 的多个 topology-node
//! fact 合并成一个 canonical 节点,并 remap 边端点。这样 code-repo 的 `BUILDS` 边
//! (repo -> `image-ref:<ref>` 节点)能合并到 k8s 的 `image:{c}:{ns}:{ref}` 节点
//! (两者都带 `image-ref:<ref>` key)→ repo->image->container->pod 联通。
//!
//! `correlation_keys` 走 `attributes_json`(JSON 字符串数组),**不新增 Fact/ResolvedNode
//! 字段** → WIT record / Arrow schema / Parquet / SQLite 列零改。
//!
//! ## 决定性是 load-bearing 不变量
//!
//! [`crate::changeset::diff`] 按 `attributes_json` 字符串相等 + `resource_id` 比
//! `ResolvedNode`(全结构 `Eq`)。故合并必须是与输入顺序无关的纯函数:
//! - **winner** = max `source_priority`,平局 lex-min `resource_id`;
//! - **attr 合并** 用 serde_json `Map`(默认 BTreeMap-backed)→ `to_string()` 产
//!   canonical 有序 key 串(winner 优先,loser 补 winner 缺失的 key);
//! - 首次合并 loser rid 会进一次 `nodes_removed`,之后稳定(非复发)。

use std::collections::{HashMap, HashSet};

use engine_core::Fact;
use serde_json::{Map, Value};

/// 源优先级 —— 决定 canonical winner(runtime 源 k8s 高于声明源 code-repo)。
///
/// 平局再按 lex-min `resource_id`。决定性 + 顺序无关。可扩展(v2 加 confidence 时
/// 这里换算分)。
fn source_priority(source: &str) -> u32 {
    match source {
        "k8s" => 10,
        "code-repo" => 5,
        _ => 0,
    }
}

/// 从 node fact 的 attributes_json 读 `correlation_keys`(字符串数组)。无 / 非法 → 空。
fn correlation_keys(attr: &Value) -> Vec<String> {
    attr.get("correlation_keys")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// 解 `attributes_json` 成 `Map`;非法 / 非 object → 空 map。
fn parse_map(attr_json: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(attr_json) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// `resolve()` 预处理:合并共享 correlation key 的 topology-node fact,remap 边端点
/// + `parent_resource_id`。无 key / 无合并 → 原样返回(零成本短路)。
///
/// 输出保持输入顺序(clustered winner 在其首次出现处发合并节点,仅一次;loser 丢;
/// 非簇 node remap parent 后发;edge remap 端点;metric/change/alert 直通)。
pub fn rewrite_by_correlation_key(facts: &[Fact]) -> Vec<Fact> {
    // 1. 收集 node fact 的 correlation keys(rid -> keys)。
    let mut rid_keys: HashMap<String, Vec<String>> = HashMap::new();
    for f in facts {
        if f.kind != "topology-node" {
            continue;
        }
        let attr = serde_json::from_str::<Value>(&f.attributes_json).unwrap_or(Value::Null);
        let keys = correlation_keys(&attr);
        if !keys.is_empty() {
            rid_keys.entry(f.resource_id.clone()).or_default().extend(keys);
        }
    }
    if rid_keys.is_empty() {
        return facts.to_vec();
    }

    // 2. key -> rids 反查。
    let mut key_rids: HashMap<String, Vec<String>> = HashMap::new();
    for (rid, keys) in &rid_keys {
        for k in keys {
            key_rids.entry(k.clone()).or_default().push(rid.clone());
        }
    }

    // 3. BFS 聚簇:经共享 key 连通的 rid 归一簇(支持传递合并 A-K1-B-K2-C)。
    let mut visited: HashSet<String> = HashSet::new();
    let mut clusters: Vec<Vec<String>> = Vec::new();
    for start in rid_keys.keys() {
        if visited.contains(start) {
            continue;
        }
        let mut stack = vec![start.clone()];
        let mut cluster: Vec<String> = Vec::new();
        while let Some(rid) = stack.pop() {
            if !visited.insert(rid.clone()) {
                continue;
            }
            cluster.push(rid.clone());
            if let Some(keys) = rid_keys.get(&rid) {
                for k in keys {
                    if let Some(neighbors) = key_rids.get(k) {
                        for n in neighbors {
                            if !visited.contains(n) {
                                stack.push(n.clone());
                            }
                        }
                    }
                }
            }
        }
        clusters.push(cluster);
    }

    // 4. rid -> best source(最高 priority)+ rid -> base fact(max timestamp)。
    let mut rid_source: HashMap<String, String> = HashMap::new();
    let mut rid_base: HashMap<String, &Fact> = HashMap::new();
    for f in facts {
        if f.kind != "topology-node" {
            continue;
        }
        let pri = source_priority(&f.source);
        match rid_source.get(&f.resource_id) {
            Some(prev) if source_priority(prev) >= pri => {}
            _ => {
                rid_source.insert(f.resource_id.clone(), f.source.clone());
            }
        }
        match rid_base.get(&f.resource_id) {
            Some(prev) if prev.timestamp >= f.timestamp => {}
            _ => {
                rid_base.insert(f.resource_id.clone(), f);
            }
        }
    }

    // 5. 每簇(>1 成员)选 winner + 建 remap(loser->winner)+ 产合并 node fact。
    let mut remap: HashMap<String, String> = HashMap::new();
    let mut winner_of: HashMap<String, String> = HashMap::new();
    let mut merged: HashMap<String, Fact> = HashMap::new();
    for cluster in &clusters {
        if cluster.len() < 2 {
            continue;
        }
        let winner = cluster
            .iter()
            .min_by(|a, b| {
                let pa = source_priority(rid_source.get(a.as_str()).map(String::as_str).unwrap_or(""));
                let pb = source_priority(rid_source.get(b.as_str()).map(String::as_str).unwrap_or(""));
                pb.cmp(&pa).then_with(|| a.cmp(b)) // higher priority first; tie -> lex-min rid
            })
            .cloned()
            .unwrap();
        for m in cluster {
            winner_of.insert(m.clone(), winner.clone());
            if *m != winner {
                remap.insert(m.clone(), winner.clone());
            }
        }
        // 合并 node fact:winner base attrs + loser 补 winner 缺失 key(canonical 序)。
        if let Some(base) = rid_base.get(&winner).copied() {
            let mut attrs = parse_map(&base.attributes_json);
            for m in cluster {
                if *m == winner {
                    continue;
                }
                if let Some(lf) = rid_base.get(m).copied() {
                    for (k, v) in parse_map(&lf.attributes_json) {
                        attrs.entry(k).or_insert(v);
                    }
                }
            }
            let mut fact = (*base).clone();
            fact.attributes_json = Value::Object(attrs).to_string();
            merged.insert(winner.clone(), fact);
        }
    }
    if remap.is_empty() {
        return facts.to_vec();
    }

    // 6. 输出(保输入顺序):clustered winner 首次出现处发合并节点(仅一次);loser 丢;
    //    非簇 node remap parent;edge remap 端点;其余直通。
    let mut emitted: HashSet<String> = HashSet::new();
    let mut out: Vec<Fact> = Vec::with_capacity(facts.len());
    for f in facts {
        match f.kind.as_str() {
            "topology-node" => {
                if let Some(winner) = winner_of.get(&f.resource_id) {
                    if f.resource_id == *winner && emitted.insert(winner.clone()) {
                        if let Some(m) = merged.get(winner) {
                            out.push(m.clone());
                        }
                    }
                    // loser: 丢(已并入 winner)
                } else {
                    out.push(remap_node_parent(f, &remap));
                }
            }
            "topology-edge" => out.push(remap_edge(f, &remap)),
            _ => out.push(f.clone()),
        }
    }
    out
}

/// remap topology-edge fact 的 `source`/`target`(若在 remap 中)+ 重建 `resource_id`。
fn remap_edge(f: &Fact, remap: &HashMap<String, String>) -> Fact {
    let mut attrs = parse_map(&f.attributes_json);
    let mut changed = false;
    for field in ["source", "target"] {
        let Some(v) = attrs.get(field).and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(winner) = remap.get(&v) {
            attrs.insert(field.to_string(), Value::String(winner.clone()));
            changed = true;
        }
    }
    if !changed {
        return f.clone();
    }
    let source = attrs.get("source").and_then(Value::as_str).unwrap_or("").to_string();
    let target = attrs.get("target").and_then(Value::as_str).unwrap_or("").to_string();
    let edge_type = attrs
        .get("edge_type")
        .and_then(Value::as_str)
        .unwrap_or("RELATED_TO")
        .to_string();
    let mut out = f.clone();
    out.attributes_json = Value::Object(attrs).to_string();
    out.resource_id = format!("edge:{edge_type}:{source}->{target}");
    out
}

/// remap topology-node fact 的 `parent_resource_id`(若父在 remap 中)。
fn remap_node_parent(f: &Fact, remap: &HashMap<String, String>) -> Fact {
    let mut attrs = parse_map(&f.attributes_json);
    let Some(v) = attrs
        .get("parent_resource_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return f.clone();
    };
    let Some(winner) = remap.get(&v) else {
        return f.clone();
    };
    attrs.insert(
        "parent_resource_id".to_string(),
        Value::String(winner.clone()),
    );
    let mut out = f.clone();
    out.attributes_json = Value::Object(attrs).to_string();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(source: &str, rid: &str, rtype: &str, attrs: Value) -> Fact {
        Fact {
            id: format!("{source}:{rid}"),
            kind: "topology-node".to_string(),
            source: source.to_string(),
            resource_id: rid.to_string(),
            resource_type: rtype.to_string(),
            timestamp: 1000,
            attributes_json: attrs.to_string(),
        }
    }

    fn edge(source: &str, et: &str, src: &str, tgt: &str) -> Fact {
        Fact {
            id: format!("{source}:edge:{et}:{src}->{tgt}"),
            kind: "topology-edge".to_string(),
            source: source.to_string(),
            resource_id: format!("edge:{et}:{src}->{tgt}"),
            resource_type: "Edge".to_string(),
            timestamp: 1000,
            attributes_json: json!({ "source": src, "target": tgt, "edge_type": et }).to_string(),
        }
    }

    #[test]
    fn merges_nodes_sharing_key_k8s_wins() {
        let k8s = node(
            "k8s",
            "image:c:ns:cart:1.0",
            "ContainerImage",
            json!({ "image": "cart:1.0", "cluster": "c", "correlation_keys": ["image-ref:cart:1.0"] }),
        );
        let repo = node(
            "code-repo",
            "image-ref:cart:1.0",
            "ContainerImage",
            json!({ "image": "cart:1.0", "correlation_keys": ["image-ref:cart:1.0"] }),
        );
        let out = rewrite_by_correlation_key(&[k8s, repo]);
        let nodes: Vec<&Fact> = out.iter().filter(|f| f.kind == "topology-node").collect();
        assert_eq!(nodes.len(), 1, "merged to one node");
        assert_eq!(nodes[0].resource_id, "image:c:ns:cart:1.0", "k8s wins (higher priority)");
    }

    #[test]
    fn remaps_edge_endpoint_onto_winner() {
        let k8s = node("k8s", "image:c:ns:cart:1.0", "ContainerImage", json!({ "correlation_keys": ["image-ref:cart:1.0"] }));
        let repo_node = node("code-repo", "image-ref:cart:1.0", "ContainerImage", json!({ "correlation_keys": ["image-ref:cart:1.0"] }));
        let builds = edge("code-repo", "BUILDS", "repo:x:cart", "image-ref:cart:1.0");
        let out = rewrite_by_correlation_key(&[k8s, repo_node, builds]);
        let edges: Vec<&Fact> = out.iter().filter(|f| f.kind == "topology-edge").collect();
        assert_eq!(edges.len(), 1);
        let attrs: Value = serde_json::from_str(&edges[0].attributes_json).unwrap();
        assert_eq!(attrs["target"], "image:c:ns:cart:1.0", "edge remapped onto k8s winner");
        assert_eq!(attrs["edge_type"], "BUILDS");
        assert!(edges[0].resource_id.contains("image:c:ns:cart:1.0"));
    }

    #[test]
    fn orphan_keyed_node_passes_through() {
        // 单成员簇(无其它节点共享 key)→ 不合并,原样透传(BUILDS 仍挂此节点,不悬空)。
        let n = node("code-repo", "image-ref:lonely:1.0", "ContainerImage", json!({ "correlation_keys": ["image-ref:lonely:1.0"] }));
        let out = rewrite_by_correlation_key(std::slice::from_ref(&n));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].resource_id, "image-ref:lonely:1.0");
    }

    #[test]
    fn merged_attrs_winner_precedence_loser_fills_missing() {
        let k8s = node("k8s", "image:c:ns:x:1", "ContainerImage", json!({ "image": "x:1", "cluster": "c", "correlation_keys": ["image-ref:x:1"] }));
        let repo = node("code-repo", "image-ref:x:1", "ContainerImage", json!({ "image": "winner-should-keep", "git_url": "https://x", "correlation_keys": ["image-ref:x:1"] }));
        let out = rewrite_by_correlation_key(&[k8s, repo]);
        let n = out.iter().find(|f| f.kind == "topology-node").unwrap();
        let a: Value = serde_json::from_str(&n.attributes_json).unwrap();
        assert_eq!(a["cluster"], "c", "winner attr kept");
        assert_eq!(a["git_url"], "https://x", "loser fills missing key");
        assert_eq!(a["image"], "x:1", "winner precedence on shared key");
    }

    #[test]
    fn no_key_facts_pass_through() {
        let pod = node("k8s", "pod:c:ns:p1", "Pod", json!({ "name": "p1" }));
        let out = rewrite_by_correlation_key(std::slice::from_ref(&pod));
        assert_eq!(out, vec![pod]);
    }

    #[test]
    fn deterministic_under_input_shuffle() {
        let k8s = node("k8s", "image:c:ns:x:1", "ContainerImage", json!({ "image": "x:1", "correlation_keys": ["image-ref:x:1"] }));
        let repo = node("code-repo", "image-ref:x:1", "ContainerImage", json!({ "git_url": "u", "correlation_keys": ["image-ref:x:1"] }));
        let a = rewrite_by_correlation_key(&[k8s.clone(), repo.clone()]);
        let b = rewrite_by_correlation_key(&[repo, k8s]);
        let mut ra: Vec<String> = a.iter().map(|f| f.resource_id.clone()).collect();
        ra.sort();
        let mut rb: Vec<String> = b.iter().map(|f| f.resource_id.clone()).collect();
        rb.sort();
        assert_eq!(ra, rb, "same node set regardless of input order");
        // 合并节点 attrs 字节一致(diff-stable)
        let am = a.iter().find(|f| f.kind == "topology-node").unwrap().attributes_json.clone();
        let bm = b.iter().find(|f| f.kind == "topology-node").unwrap().attributes_json.clone();
        assert_eq!(am, bm, "merged attrs byte-identical");
    }

    #[test]
    fn resolve_attaches_builds_to_k8s_image_via_correlation_key() {
        // C1 收官证明:code-repo BUILDS(repo -> image-ref node)经 correlation key
        // 合并到 k8s image 节点,BUILDS 边端点 remap 到 k8s image id → 不再悬空。
        use crate::resolve;
        let k8s_img = node("k8s", "image:vm:otel:cart:1.0", "ContainerImage", json!({ "image": "cart:1.0", "cluster": "vm", "correlation_keys": ["image-ref:cart:1.0"] }));
        let repo = node("code-repo", "repo:gh:otel:cart", "CodeRepo", json!({ "name": "cart" }));
        let img_ref = node("code-repo", "image-ref:cart:1.0", "ContainerImage", json!({ "image": "cart:1.0", "correlation_keys": ["image-ref:cart:1.0"] }));
        let builds = edge("code-repo", "BUILDS", "repo:gh:otel:cart", "image-ref:cart:1.0");
        let topo = resolve(&[k8s_img, repo, img_ref, builds]);

        let imgs: Vec<_> = topo.nodes.iter().filter(|n| n.resource_type == "ContainerImage").collect();
        assert_eq!(imgs.len(), 1, "image-ref merged into k8s image (1 node, not 2)");
        assert_eq!(imgs[0].resource_id, "image:vm:otel:cart:1.0");

        let builds_edge = topo.edges.iter().find(|e| e.edge_type == "BUILDS").expect("BUILDS present");
        assert_eq!(builds_edge.target, "image:vm:otel:cart:1.0", "BUILDS attached to k8s image (not dangling)");
        assert_eq!(builds_edge.source, "repo:gh:otel:cart");
    }
}
