//! Canonical `resource_type` / `edge_type` vocabulary — host-side single source.
//!
//! These strings flow through the system as wire-format text (`Fact.resource_type`,
//! `edge.edge_type`, `attributes_json`), so the registry is `&'static str` consts rather
//! than an enum. **Host-side consumers (views / recovery / changes / desktop selectors)
//! must reference these consts** instead of bare string literals, so a rename is a
//! single-point change and typos surface as "unknown type" in the validation test below.
//!
//! ## Guest / host split (inherent limitation)
//!
//! `modules/connectors/*` is a **separate WASM workspace** that depends on `module-sdk`,
//! not `engine-core`. Guest mappers therefore emit these strings as literals and **cannot
//! import this registry**. Drift between guest output and this host registry is caught by:
//!
//! 1. The realistic-fixture test in `engine-identity/src/views.rs` (uses these consts).
//! 2. The `engine-inspect-views` headless tool against a live SQLite materialized topology.
//!
//! This is the exact failure mode of the Node Impact bug (reference Neo4j label
//! `KubernetesNode` vs Rust `resource_type` `Node`). The registry centralizes the host
//! spelling; the two detectors above cover the guest boundary.
//!
//! A future codegen-from-WIT single source would close the guest/host gap entirely.

/// Canonical `resource_type` strings.
///
/// Consumed by views (start-node selectors), recovery/changes (type filters), reports
/// (gatherers), and the desktop frontend. Emitted by guest connectors as
/// `Fact.resource_type` — see module docs on the guest/host split.
#[allow(missing_docs)] // names are self-documenting; vocabulary documented at module level
pub mod resource_type {
    // --- K8s structural types (modules/connectors/k8s) ---
    pub const CLUSTER: &str = "Cluster";
    pub const NAMESPACE: &str = "Namespace";
    pub const NODE: &str = "Node";
    pub const POD: &str = "Pod";
    pub const DEPLOYMENT: &str = "Deployment";
    pub const SERVICE: &str = "Service";
    pub const CONFIG_MAP: &str = "ConfigMap";
    pub const SECRET: &str = "Secret";
    pub const CONTAINER: &str = "Container";
    pub const CONTAINER_IMAGE: &str = "ContainerImage";

    // --- Derived application / component / middleware layer (Phase 3.8) ---
    pub const APPLICATION: &str = "Application";
    pub const APPLICATION_COMPONENT: &str = "ApplicationComponent";
    pub const REDIS: &str = "Redis";
    pub const KAFKA: &str = "Kafka";
    pub const MYSQL: &str = "MySQL";

    // --- L3 dynamic (synthesized by engine-changes::alert_aggregation) ---
    pub const ALERT_EVENT: &str = "AlertEvent";

    /// All host-recognized resource types (canonical vocabulary).
    pub const ALL: &[&str] = &[
        CLUSTER,
        NAMESPACE,
        NODE,
        POD,
        DEPLOYMENT,
        SERVICE,
        CONFIG_MAP,
        SECRET,
        CONTAINER,
        CONTAINER_IMAGE,
        APPLICATION,
        APPLICATION_COMPONENT,
        REDIS,
        KAFKA,
        MYSQL,
        ALERT_EVENT,
    ];

    /// True iff `t` is a recognized resource type.
    pub fn is_known(t: &str) -> bool {
        ALL.contains(&t)
    }
}

/// Canonical `edge_type` strings.
///
/// Split into **produced** (some connector / resolver emits them in a real topology) and
/// **whitelist-only** (reference Cypher uses them; Rust does not produce them yet — kept
/// so view traversals stay future-proof and harmlessly match nothing today).
#[allow(missing_docs)] // names are self-documenting; vocabulary documented at module level
pub mod edge_type {
    // --- Produced by connectors / resolution ---
    pub const CONTAINS: &str = "CONTAINS";
    pub const BELONGS_TO: &str = "BELONGS_TO";
    pub const DEPLOYED_AS: &str = "DEPLOYED_AS";
    pub const SCHEDULED_ON: &str = "SCHEDULED_ON";
    pub const ROUTES_TO: &str = "ROUTES_TO";
    pub const EXPOSES: &str = "EXPOSES";
    pub const RUNS: &str = "RUNS";
    pub const USES: &str = "USES";
    pub const USES_IMAGE: &str = "USES_IMAGE";
    pub const FIRED_ON: &str = "FIRED_ON"; // synthesized by alert_aggregation
    pub const CALLS: &str = "CALLS"; // jaeger trace connector (CHILD_OF aggregation)

    /// Edge types currently produced by some connector / resolver.
    pub const PRODUCED: &[&str] = &[
        CONTAINS,
        BELONGS_TO,
        DEPLOYED_AS,
        SCHEDULED_ON,
        ROUTES_TO,
        EXPOSES,
        RUNS,
        USES,
        USES_IMAGE,
        FIRED_ON,
        CALLS,
    ];

    // --- Whitelist-only (reference Cypher; Rust not yet producing — future-proof) ---
    pub const CONTROLLED_BY: &str = "CONTROLLED_BY";
    pub const AFFECTS: &str = "AFFECTS";
    pub const STORED_IN: &str = "STORED_IN";
    pub const DEPLOYED_IN: &str = "DEPLOYED_IN";

    /// All host-recognized edge types (produced + reference-only future).
    pub const KNOWN: &[&str] = &[
        CONTAINS,
        BELONGS_TO,
        DEPLOYED_AS,
        SCHEDULED_ON,
        ROUTES_TO,
        EXPOSES,
        RUNS,
        USES,
        USES_IMAGE,
        FIRED_ON,
        CONTROLLED_BY,
        AFFECTS,
        STORED_IN,
        DEPLOYED_IN,
        CALLS,
    ];

    /// True iff `e` is a recognized edge type.
    pub fn is_known(e: &str) -> bool {
        KNOWN.contains(&e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_type_all_is_unique() {
        let mut sorted = resource_type::ALL.to_vec();
        sorted.sort();
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(sorted, deduped, "resource_type::ALL has duplicates");
    }

    #[test]
    fn edge_type_known_is_unique_and_superset_of_produced() {
        let mut sorted = edge_type::KNOWN.to_vec();
        sorted.sort();
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(sorted, deduped, "edge_type::KNOWN has duplicates");

        for p in edge_type::PRODUCED {
            assert!(
                edge_type::KNOWN.contains(p),
                "PRODUCED edge {p} not in KNOWN"
            );
        }
    }

    #[test]
    fn is_known_helpers() {
        assert!(resource_type::is_known("Node"));
        assert!(!resource_type::is_known("KubernetesNode")); // the bug
        assert!(edge_type::is_known("USES_IMAGE"));
        assert!(!edge_type::is_known("USES-IMAGE"));
    }

    /// Const names must equal their string values — guards against a copy-paste where
    /// `pub const NODE: &str = "Pod";`. Each const's identifier (SCREAMING_SNAKE) maps to
    /// its PascalCase / SCREAMING value.
    #[test]
    fn const_names_match_values() {
        // resource_type: PascalCase value == const name title-cased
        assert_eq!(resource_type::NODE, "Node");
        assert_eq!(resource_type::APPLICATION_COMPONENT, "ApplicationComponent");
        assert_eq!(resource_type::CONTAINER_IMAGE, "ContainerImage");
        assert_eq!(resource_type::ALERT_EVENT, "AlertEvent");
        // edge_type: SCREAMING value == const name exactly
        assert_eq!(edge_type::CONTAINS, "CONTAINS");
        assert_eq!(edge_type::USES_IMAGE, "USES_IMAGE");
        assert_eq!(edge_type::SCHEDULED_ON, "SCHEDULED_ON");
    }
}
