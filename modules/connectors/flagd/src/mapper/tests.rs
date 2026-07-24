//! mapper 纯函数单测 —— 移植 reference `TestFlagdConnector` / `TestFlagdScenarioEnrichment`
//! / `TestScenarios`。host target,CI-safe。

use super::*;
use serde_json::json;

fn cfg() -> Cfg {
    Cfg::new("vm-cluster", "otel-demo", "otel-demo-flagd-config", 1_700_000_000)
}

fn snap(pairs: &[(&str, Value)]) -> Snapshot {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn fact_attr(f: &Fact, pointer: &str) -> Value {
    let v: Value = serde_json::from_str(&f.attributes_json).unwrap();
    v.pointer(pointer).cloned().unwrap_or(Value::Null)
}

// ── extract_value / state_differs(对照 TestFlagdConnector 前 4 例)──

#[test]
fn extract_value_bool() {
    assert_eq!(extract_value(&json!({"variant":"off","boolValue":false})), json!(false));
    assert_eq!(extract_value(&json!({"variant":"on","boolValue":true})), json!(true));
}

#[test]
fn extract_value_double() {
    assert_eq!(extract_value(&json!({"variant":"level1","doubleValue":0.5})), json!(0.5));
}

#[test]
fn state_differs_by_variant() {
    let old = json!({"variant":"off","boolValue":false});
    let new = json!({"variant":"on","boolValue":true});
    assert!(state_differs(&old, &new));
}

#[test]
fn state_differs_same_returns_false() {
    let s = json!({"variant":"on","boolValue":true});
    assert!(!state_differs(&s, &s));
}

// ── diff_snapshots:changed / added / removed(对照 5-8 例)──

#[test]
fn diff_emits_changed_on_flip() {
    let old = snap(&[("productCatalogFailure", json!({"variant":"off","boolValue":false}))]);
    let new = snap(&[("productCatalogFailure", json!({"variant":"on","boolValue":true}))]);
    let deltas = diff_snapshots(&old, &new);
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].flag_name, "productCatalogFailure");
    assert_eq!(deltas[0].old, Some(json!(false)));
    assert_eq!(deltas[0].new, Some(json!(true)));
    assert!(deltas[0].description.contains("variant=off"));
}

#[test]
fn diff_emits_added_for_new_flag() {
    let old = Snapshot::new();
    let new = snap(&[("someFlag", json!({"variant":"on","boolValue":true}))]);
    let deltas = diff_snapshots(&old, &new);
    assert_eq!(deltas.len(), 1);
    assert!(deltas[0].old.is_none());
    assert_eq!(deltas[0].new, Some(json!(true)));
    assert!(deltas[0].description.contains("flag added"));
}

#[test]
fn diff_emits_removed_for_gone_flag() {
    let old = snap(&[("goneFlag", json!({"variant":"on","boolValue":true}))]);
    let new = Snapshot::new();
    let deltas = diff_snapshots(&old, &new);
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].old, Some(json!(true)));
    assert!(deltas[0].new.is_none());
    assert!(deltas[0].description.contains("flag removed"));
}

#[test]
fn diff_no_change_when_identical() {
    let s = snap(&[("x", json!({"variant":"on","boolValue":true}))]);
    assert!(diff_snapshots(&s, &s).is_empty());
}

// ── scenario enrichment(对照 TestFlagdScenarioEnrichment)──

#[test]
fn fault_flag_event_carries_scenario() {
    let old = snap(&[("productCatalogFailure", json!({"variant":"off","boolValue":false}))]);
    let new = snap(&[("productCatalogFailure", json!({"variant":"on","boolValue":true}))]);
    let f = delta_to_change_fact(&diff_snapshots(&old, &new)[0], &cfg());
    assert_eq!(f.kind, "change");
    assert_eq!(fact_attr(&f, "/change_type"), json!("configmap_updated"));
    assert_eq!(fact_attr(&f, "/source"), json!("flagd"));
    assert_eq!(
        fact_attr(&f, "/target_resource_id"),
        json!("configmap:vm-cluster:otel-demo:otel-demo-flagd-config")
    );
    assert_eq!(fact_attr(&f, "/diff_summary/scenario/recommended_action"), json!("restart_pod"));
    assert_eq!(fact_attr(&f, "/diff_summary/scenario/target_component"), json!("product-catalog"));
    let desc = fact_attr(&f, "/description").as_str().unwrap().to_string();
    assert!(desc.contains("scenario=") && desc.contains("restart_pod"), "desc={desc}");
}

#[test]
fn non_fault_flag_has_no_scenario() {
    let old = snap(&[("someBusinessToggle", json!({"variant":"off","boolValue":false}))]);
    let new = snap(&[("someBusinessToggle", json!({"variant":"on","boolValue":true}))]);
    let f = delta_to_change_fact(&diff_snapshots(&old, &new)[0], &cfg());
    assert_eq!(fact_attr(&f, "/diff_summary/scenario"), Value::Null);
    let desc = fact_attr(&f, "/description").as_str().unwrap().to_string();
    assert!(!desc.contains("scenario="));
}

#[test]
fn lookup_scenario_unknown_returns_none() {
    assert!(scenario_for_flag("ghost-flag").is_none());
}

#[test]
fn lookup_scenario_known() {
    let s = scenario_for_flag("cartServiceFailure").expect("cart scenario");
    assert_eq!(s.recommended_action, "clear_cache");
    assert_eq!(scenario_for_name("cart_failure").unwrap().flag_name, "cartServiceFailure");
}

// ── scenario 表完整性(对照 TestScenarios)──

#[test]
fn at_least_seven_scenarios() {
    assert!(SCENARIOS.len() >= 7);
    assert_eq!(SCENARIOS.len(), 8); // reference 实际 8
}

#[test]
fn all_scenarios_have_required_fields() {
    for s in SCENARIOS {
        assert!(!s.flag_name.is_empty(), "empty flag_name");
        assert!(!s.target_component.is_empty(), "empty target_component");
        assert!(
            VALID_RECOMMENDED_ACTIONS.contains(&s.recommended_action),
            "bad recommended_action: {}",
            s.recommended_action
        );
        assert!(matches!(s.finding_severity, "warning" | "critical"), "bad severity: {}", s.finding_severity);
    }
}

#[test]
fn scenario_names_and_flags_unique() {
    let mut names: Vec<_> = SCENARIOS.iter().map(|s| s.name).collect();
    names.sort();
    let mut flags: Vec<_> = SCENARIOS.iter().map(|s| s.flag_name).collect();
    flags.sort();
    let nd = { let mut v = names.clone(); v.dedup(); v.len() };
    let fd = { let mut v = flags.clone(); v.dedup(); v.len() };
    assert_eq!(names.len(), nd, "duplicate scenario names");
    assert_eq!(flags.len(), fd, "duplicate flag names");
}
