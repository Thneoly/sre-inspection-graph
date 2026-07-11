//! YAML diff 工具(复刻 `reference/app/changes/yaml_diff.py`)。
//!
//! 把两个 K8s 资源对象对比成 unified diff 文本,供 `ChangeEvent.yaml_diff` 存档 +
//! 前端 `<pre>` 渲染。默认剔除 K8s 噪声字段(资源版本 / uid / managedFields 等),
//! 避免每次变更都报一堆元数据 diff。
//!
//! ## 与 reference 的差异
//!
//! - **YAML 序列化**:reference 用 `yaml.safe_dump(sort_keys=True)`;本 port 用自写的
//!   确定性 block-style 发射器(`BTreeMap` 排序键),**不引 serde_yaml**(避免新依赖 +
//!   网络拉取)。字符串不加引号(reference 按需单引号) -- 契约测试是子串断言,不依赖
//!   严格 YAML 文本;diff=="" 由剥离噪声后的 [`serde_json::Value`] 结构相等短路保证。
//! - **unified diff**:reference 用 `difflib.unified_diff`;本 port 用 `similar` crate
//!   (已在 Cargo.lock 缓存)。hunk 格式可能略有差异,但 `+`/`-`/` ` 前缀语义一致,
//!   [`summarize_diff`] 据此前缀统计。
//! - 算法逐字对齐:递归剥噪声、顶层 `keys` 限定、空串短路、`summarize_diff` 的
//!   `added`/`removed`/`changed_keys` 启发式。

#![allow(missing_docs)]

use serde_json::{Map, Value};
use similar::{Algorithm, udiff::unified_diff};

/// K8s 对象里纯元数据、变更无业务意义的字段 -- diff 前剔除(对齐 reference `_NOISE_KEYS`)。
pub const NOISE_KEYS: &[&str] = &[
    "managedFields",
    "resourceVersion",
    "uid",
    "creationTimestamp",
    "generation",
    "selfLink",
    "etag",
    "last-applied-configuration",
    "annotations",
    "managedVersion",
];

/// diff 统计(对齐 reference `summarize_diff` 返回)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DiffSummary {
    /// `+` 行数(不含 `+++` 头)。
    pub added: usize,
    /// `-` 行数(不含 `---` 头)。
    pub removed: usize,
    /// 去重排序后的变更顶层 key(启发式提取)。
    pub changed_keys: Vec<String>,
}

/// 递归剔除噪声字段(dict 层级;对齐 reference `_strip_noise`)。
pub fn strip_noise(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let noise: std::collections::HashSet<&str> = NOISE_KEYS.iter().copied().collect();
            let filtered: Map<String, Value> = map
                .iter()
                .filter(|(k, _)| !noise.contains(k.as_str()))
                .map(|(k, v)| (k.clone(), strip_noise(v)))
                .collect();
            Value::Object(filtered)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(strip_noise).collect()),
        other => other.clone(),
    }
}

/// 只保留指定顶层 key;`keys=None` 全保留(对齐 reference `_select_keys`)。
pub fn select_keys(value: &Value, keys: Option<&[&str]>) -> Value {
    match (value, keys) {
        (Value::Object(map), Some(wanted)) => {
            let want: std::collections::HashSet<&str> = wanted.iter().copied().collect();
            let filtered: Map<String, Value> = map
                .iter()
                .filter(|(k, _)| want.contains(k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Value::Object(filtered)
        }
        _ => value.clone(),
    }
}

/// 对比两个对象 -> unified diff 文本(对齐 reference `compute_yaml_diff`)。
///
/// - `old`/`new` 为 `Null`/空对象视为新增/删除。
/// - `keys` 限定只对比某些顶层 key(如 `["data", "spec"]`)。
/// - 返回空串表示无业务差异(剥离噪声 + 选键后一致)。
pub fn compute_yaml_diff(old: &Value, new: &Value, keys: Option<&[&str]>, name: &str) -> String {
    let old_stripped = strip_noise(&select_keys(&coerce_object(old), keys));
    let new_stripped = strip_noise(&select_keys(&coerce_object(new), keys));

    // 结构相等(对象键序无关)直接短路 -> ""(对齐 reference `old_yaml == new_yaml`)
    if old_stripped == new_stripped {
        return String::new();
    }

    let old_yaml = emit_yaml(&old_stripped);
    let new_yaml = emit_yaml(&new_stripped);

    let left = format!("{name}.old");
    let right = format!("{name}.new");
    let patch = unified_diff(Algorithm::default(), &old_yaml, &new_yaml, 3, Some((&left, &right)));
    patch.trim_end_matches('\n').to_string()
}

/// 从 unified diff 文本解析统计(对齐 reference `summarize_diff`)。
pub fn summarize_diff(diff_text: &str) -> DiffSummary {
    if diff_text.is_empty() {
        return DiffSummary { added: 0, removed: 0, changed_keys: Vec::new() };
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut changed_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for line in diff_text.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            added += 1;
            if let Some(k) = extract_key(rest) {
                changed_keys.insert(k);
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            removed += 1;
            if let Some(k) = extract_key(rest) {
                changed_keys.insert(k);
            }
        }
    }
    DiffSummary {
        added,
        removed,
        changed_keys: changed_keys.into_iter().collect(),
    }
}

// ===== helpers =====

/// `Null` -> 空对象;其它非对象原样(对齐 reference `old_obj or {}`)。
fn coerce_object(v: &Value) -> Value {
    if v.is_null() {
        Value::Object(Map::new())
    } else {
        v.clone()
    }
}

/// 从 YAML 行提取顶层 key(对齐 reference `_extract_key`)。
///
/// 粗粒度:取第一个非空白、非 `-`/`#` 的 `:` 前部分。嵌套 key 不展开。
fn extract_key(line: &str) -> Option<String> {
    let stripped = line.trim_start();
    if stripped.is_empty() || stripped.starts_with('-') || stripped.starts_with('#') {
        return None;
    }
    let colon = stripped.find(':')?;
    let key = stripped[..colon].trim();
    if key.is_empty() {
        None
    } else {
        Some(key.to_string())
    }
}

/// 确定性 block-style YAML 发射器(排序键,对齐 `yaml.safe_dump(sort_keys=True)`)。
///
/// 字符串不加引号(契约测试子串断言不依赖);空对象 -> `{}`、空数组 -> `[]`(inline)。
fn emit_yaml(value: &Value) -> String {
    let mut out = String::new();
    emit_value(value, 0, &mut out);
    out
}

fn emit_value(value: &Value, indent: usize, out: &mut String) {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<&String, &Value> = map.iter().collect();
            for (k, v) in &sorted {
                emit_entry(k, v, indent, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                emit_array_item(item, indent, out);
            }
        }
        other => {
            out.push_str(&emit_scalar(other));
            out.push('\n');
        }
    }
}

fn emit_entry(key: &str, value: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(inner) if inner.is_empty() => {
            out.push_str(&format!("{pad}{key}: {{}}\n"));
        }
        Value::Array(inner) if inner.is_empty() => {
            out.push_str(&format!("{pad}{key}: []\n"));
        }
        Value::Object(_) | Value::Array(_) => {
            out.push_str(&format!("{pad}{key}:\n"));
            emit_value(value, indent + 2, out);
        }
        scalar => {
            out.push_str(&format!("{pad}{key}: {}\n", emit_scalar(scalar)));
        }
    }
}

fn emit_array_item(item: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match item {
        Value::Object(inner) if inner.is_empty() => {
            out.push_str(&format!("{pad}- {{}}\n"));
        }
        Value::Array(inner) if inner.is_empty() => {
            out.push_str(&format!("{pad}- []\n"));
        }
        Value::Object(_) | Value::Array(_) => {
            out.push_str(&format!("{pad}-\n"));
            emit_value(item, indent + 2, out);
        }
        scalar => {
            out.push_str(&format!("{pad}- {}\n", emit_scalar(scalar)));
        }
    }
}

fn emit_scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        // 复合标量兜底(理论上 emit_value 已处理,这里防递归漏网)
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compute_diff_detects_value_change() {
        let old = json!({"data": {"max_pool_size": "20", "timeout": "30"}});
        let new = json!({"data": {"max_pool_size": "50", "timeout": "30"}});
        let diff = compute_yaml_diff(&old, &new, None, "order-config");
        assert!(!diff.is_empty());
        assert!(diff.contains("max_pool_size"));
        assert!(diff.contains("20") && diff.contains("50"));
        // timeout 没变,不以 +timeout 出现
        assert!(!diff.contains("+timeout"));
    }

    #[test]
    fn compute_diff_strips_noise_fields() {
        let old = json!({
            "data": {"key": "v1"},
            "metadata": {
                "resourceVersion": "123",
                "uid": "abc-456",
                "managedFields": [{"manager": "kubectl"}],
                "creationTimestamp": "2026-01-01T00:00:00Z",
            }
        });
        let new = json!({
            "data": {"key": "v1"},
            "metadata": {
                "resourceVersion": "999",
                "uid": "abc-456",
                "managedFields": [{"manager": "kubectl", "operation": "Apply"}],
                "creationTimestamp": "2026-01-01T00:00:00Z",
            }
        });
        let diff = compute_yaml_diff(&old, &new, None, "cm");
        assert_eq!(diff, "", "noise-only change should produce empty diff, got: {diff:?}");
    }

    #[test]
    fn summarize_diff_counts_added_removed() {
        let old = json!({"data": {"a": "1", "b": "2"}});
        let new = json!({"data": {"a": "1", "b": "9", "c": "3"}});
        let diff = compute_yaml_diff(&old, &new, None, "resource");
        let summary = summarize_diff(&diff);
        assert!(summary.added >= 1, "added: {}", summary.added); // +c
        assert!(summary.removed >= 1, "removed: {}", summary.removed); // -b old value
        assert!(summary.changed_keys.contains(&"b".to_string()));
    }

    #[test]
    fn identical_objects_produce_empty_diff() {
        let old = json!({"data": {"a": "1"}});
        let new = json!({"data": {"a": "1"}});
        assert_eq!(compute_yaml_diff(&old, &new, None, "r"), "");
    }

    #[test]
    fn select_keys_limits_top_level() {
        let old = json!({"data": {"a": "1"}, "spec": {"x": "y"}});
        let new = json!({"data": {"a": "2"}, "spec": {"x": "y"}});
        // 只看 spec -> 无 diff
        assert_eq!(compute_yaml_diff(&old, &new, Some(&["spec"]), "r"), "");
        // 只看 data -> 有 diff
        assert!(!compute_yaml_diff(&old, &new, Some(&["data"]), "r").is_empty());
    }

    #[test]
    fn summarize_empty_diff_is_zero() {
        let s = summarize_diff("");
        assert_eq!(s, DiffSummary { added: 0, removed: 0, changed_keys: Vec::new() });
    }

    #[test]
    fn strip_noise_recursive() {
        let v = json!({
            "data": {"resourceVersion": "1", "real": "v"},
            "metadata": {"uid": "x", "nested": {"generation": 2, "keep": "k"}}
        });
        let stripped = strip_noise(&v);
        // 顶层 data 去掉 resourceVersion,保留 real
        assert_eq!(stripped["data"]["real"], "v");
        assert!(stripped["data"].get("resourceVersion").is_none());
        // 嵌套也剥
        assert!(stripped["metadata"].get("uid").is_none());
        assert!(stripped["metadata"]["nested"].get("generation").is_none());
        assert_eq!(stripped["metadata"]["nested"]["keep"], "k");
    }
}
