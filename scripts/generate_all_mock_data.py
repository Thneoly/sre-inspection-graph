#!/usr/bin/env python3
"""
编排脚本 — 运行所有 Mock 数据生成器 + 输出 Neo4j Cypher 导入脚本

用法:
  python scripts/generate_all_mock_data.py
"""

import os
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
OUTPUT_DIR = os.path.join(SCRIPT_DIR, "output")
os.makedirs(OUTPUT_DIR, exist_ok=True)


def run_module(module_name: str):
    """动态导入并运行生成器模块"""
    import importlib
    mod = importlib.import_module(module_name)
    mod.main()


def generate_cypher_import_script():
    """生成合并导入 Neo4j 的 Cypher 脚本"""

    cypher = """// ============================================================
// Neo4j Import Script - L3 + L4 扩展层
// 云原生巡检图谱 v1
// 自动生成于 scripts/generate_all_mock_data.py
// ============================================================
// 前置条件：已执行 L1/L2 导入脚本
// ============================================================

// ========== 0. 约束 ==========
CREATE CONSTRAINT l3_pod_id IF NOT EXISTS
FOR (n:Pod) REQUIRE n.node_id IS UNIQUE;

CREATE CONSTRAINT l3_container_id IF NOT EXISTS
FOR (n:Container) REQUIRE n.container_id IS UNIQUE;

CREATE CONSTRAINT l3_node_id IF NOT EXISTS
FOR (n:KubernetesNode) REQUIRE n.node_id IS UNIQUE;

CREATE CONSTRAINT l3_metric_snapshot_id IF NOT EXISTS
FOR (n:MetricSnapshot) REQUIRE n.snapshot_id IS UNIQUE;

CREATE CONSTRAINT l4_inspection_finding_id IF NOT EXISTS
FOR (n:InspectionFinding) REQUIRE n.node_id IS UNIQUE;

CREATE CONSTRAINT l4_alert_event_id IF NOT EXISTS
FOR (n:AlertEvent) REQUIRE n.node_id IS UNIQUE;


// ========== 1. 导入 L3 实例节点 ==========

// 1.1 KubernetesNode
LOAD CSV WITH HEADERS FROM 'file:///l3_instance_nodes.csv' AS row
WITH row WHERE row.label = 'KubernetesNode'
MERGE (n:KubernetesNode:ResourceInstance {node_id: row.node_id})
SET n.name = row.name, n.label = row.label, n.unique_key = row.unique_key,
    n.env_code = row.env_code, n.app_code = row.app_code,
    n.cluster_id = row.cluster_id, n.owner_team = row.owner_team,
    n.lifecycle_status = row.lifecycle_status, n.health_status = row.health_status,
    n.risk_level = row.risk_level, n.inspection_status = row.inspection_status,
    n.last_inspected_at = row.last_inspected_at, n.source_system = row.source_system,
    n.source_ref = row.source_ref, n.attrs_json = row.attrs_json,
    n.version = 'v1', n.updated_at = datetime();

// 1.2 Pod
LOAD CSV WITH HEADERS FROM 'file:///l3_instance_nodes.csv' AS row
WITH row WHERE row.label = 'Pod'
MERGE (n:Pod:ResourceInstance {node_id: row.node_id})
SET n.name = row.name, n.label = row.label, n.unique_key = row.unique_key,
    n.env_code = row.env_code, n.app_code = row.app_code, n.component_code = row.component_code,
    n.cluster_id = row.cluster_id, n.namespace = row.namespace, n.owner_team = row.owner_team,
    n.lifecycle_status = row.lifecycle_status, n.health_status = row.health_status,
    n.risk_level = row.risk_level, n.inspection_status = row.inspection_status,
    n.last_inspected_at = row.last_inspected_at, n.source_system = row.source_system,
    n.source_ref = row.source_ref, n.attrs_json = row.attrs_json,
    n.version = 'v1', n.updated_at = datetime();

// 1.3 Container
LOAD CSV WITH HEADERS FROM 'file:///l3_instance_nodes.csv' AS row
WITH row WHERE row.label = 'Container'
MERGE (n:Container:ResourceInstance {node_id: row.node_id})
SET n.name = row.name, n.label = row.label, n.unique_key = row.unique_key,
    n.env_code = row.env_code, n.app_code = row.app_code, n.component_code = row.component_code,
    n.cluster_id = row.cluster_id, n.namespace = row.namespace, n.owner_team = row.owner_team,
    n.lifecycle_status = row.lifecycle_status, n.health_status = row.health_status,
    n.risk_level = row.risk_level, n.inspection_status = row.inspection_status,
    n.last_inspected_at = row.last_inspected_at, n.source_system = row.source_system,
    n.source_ref = row.source_ref, n.attrs_json = row.attrs_json,
    n.version = 'v1', n.updated_at = datetime();


// ========== 2. 导入 L3 实例关系 ==========
LOAD CSV WITH HEADERS FROM 'file:///l3_instance_edges.csv' AS row
MATCH (s:ResourceInstance {node_id: row.source_node_id})
MATCH (t:ResourceInstance {node_id: row.target_node_id})
MERGE (s)-[r:RELATES_TO {edge_id: row.edge_id}]->(t)
SET r.relationship_type = row.relationship_type,
    r.relationship_name = row.relationship_name,
    r.dependency_strength = row.dependency_strength,
    r.is_required = row.is_required,
    r.discovery_method = row.discovery_method,
    r.health_status = row.health_status,
    r.risk_signal = row.risk_signal,
    r.last_verified_at = row.last_verified_at,
    r.attrs_json = row.attrs_json,
    r.version = 'v1', r.updated_at = datetime();


// ========== 3. 导入 L3 MetricQuery 和 MetricSnapshot ==========

// 3.1 MetricQuery 实例
LOAD CSV WITH HEADERS FROM 'file:///l3_metric_queries.csv' AS row
MERGE (n:MetricQuery {query_id: row.query_id})
SET n.metric_name = row.metric_name,
    n.target_resource_type = row.target_resource_type,
    n.promql_template = row.promql_template,
    n.datasource_uid = row.datasource_uid,
    n.unit = row.unit,
    n.warning_threshold = row.warning_threshold,
    n.critical_threshold = row.critical_threshold,
    n.enabled_status = row.enabled_status,
    n.datasource_status = row.datasource_status,
    n.version = 'v1', n.updated_at = datetime();

// 3.2 MetricSnapshot 实例
LOAD CSV WITH HEADERS FROM 'file:///l3_metric_snapshots.csv' AS row
MERGE (n:MetricSnapshot {snapshot_id: row.snapshot_id})
SET n.resource_id = row.resource_id,
    n.metric_name = row.metric_name,
    n.metric_query_id = row.metric_query_id,
    n.current_value = toFloat(row.current_value),
    n.unit = row.unit,
    n.fetched_at = row.fetched_at,
    n.ttl_seconds = toInteger(row.ttl_seconds),
    n.is_stale = row.is_stale,
    n.warning_breached = row.warning_breached,
    n.critical_breached = row.critical_breached,
    n.version = 'v1', n.updated_at = datetime();

// 3.3 MetricSnapshot MEASURES ResourceInstance
LOAD CSV WITH HEADERS FROM 'file:///l3_metric_snapshots.csv' AS row
MATCH (ms:MetricSnapshot {snapshot_id: row.snapshot_id})
MATCH (r:ResourceInstance {node_id: row.resource_id})
MERGE (ms)-[rel:RELATES_TO {edge_id: 'metricsnap_' + row.snapshot_id}]->(r)
SET rel.relationship_type = 'MEASURES',
    rel.relationship_name = '测量',
    rel.dependency_strength = '中',
    rel.version = 'v1', rel.updated_at = datetime();


// ========== 4. 导入 L4 实例节点 ==========

// 4.1 InspectionRun
LOAD CSV WITH HEADERS FROM 'file:///l4_instance_nodes.csv' AS row
WITH row WHERE row.label = 'InspectionRun'
MERGE (n:InspectionRun:ResourceInstance {node_id: row.node_id})
SET n.name = row.name, n.label = row.label, n.unique_key = row.unique_key,
    n.env_code = row.env_code, n.app_code = row.app_code,
    n.cluster_id = row.cluster_id, n.owner_team = row.owner_team,
    n.lifecycle_status = row.lifecycle_status, n.health_status = row.health_status,
    n.risk_level = row.risk_level, n.inspection_status = row.inspection_status,
    n.last_inspected_at = row.last_inspected_at, n.source_system = row.source_system,
    n.source_ref = row.source_ref, n.attrs_json = row.attrs_json,
    n.version = 'v1', n.updated_at = datetime();

// 4.2 InspectionRule
LOAD CSV WITH HEADERS FROM 'file:///l4_instance_nodes.csv' AS row
WITH row WHERE row.label = 'InspectionRule'
MERGE (n:InspectionRule:ResourceInstance {node_id: row.node_id})
SET n.name = row.name, n.label = row.label, n.unique_key = row.unique_key,
    n.owner_team = row.owner_team, n.lifecycle_status = row.lifecycle_status,
    n.health_status = row.health_status, n.risk_level = row.risk_level,
    n.source_system = row.source_system, n.source_ref = row.source_ref,
    n.attrs_json = row.attrs_json,
    n.version = 'v1', n.updated_at = datetime();

// 4.3 InspectionFinding
LOAD CSV WITH HEADERS FROM 'file:///l4_instance_nodes.csv' AS row
WITH row WHERE row.label = 'InspectionFinding'
MERGE (n:InspectionFinding:ResourceInstance {node_id: row.node_id})
SET n.name = row.name, n.label = row.label, n.unique_key = row.unique_key,
    n.env_code = row.env_code, n.app_code = row.app_code, n.component_code = row.component_code,
    n.cluster_id = row.cluster_id, n.namespace = row.namespace, n.owner_team = row.owner_team,
    n.lifecycle_status = row.lifecycle_status, n.health_status = row.health_status,
    n.risk_level = row.risk_level, n.inspection_status = row.inspection_status,
    n.last_inspected_at = row.last_inspected_at, n.source_system = row.source_system,
    n.source_ref = row.source_ref, n.attrs_json = row.attrs_json,
    n.version = 'v1', n.updated_at = datetime();

// 4.4 AlertEvent
LOAD CSV WITH HEADERS FROM 'file:///l4_instance_nodes.csv' AS row
WITH row WHERE row.label = 'AlertEvent'
MERGE (n:AlertEvent:ResourceInstance {node_id: row.node_id})
SET n.name = row.name, n.label = row.label, n.unique_key = row.unique_key,
    n.env_code = row.env_code, n.app_code = row.app_code, n.component_code = row.component_code,
    n.cluster_id = row.cluster_id, n.namespace = row.namespace, n.owner_team = row.owner_team,
    n.lifecycle_status = row.lifecycle_status, n.health_status = row.health_status,
    n.source_system = row.source_system, n.source_ref = row.source_ref,
    n.attrs_json = row.attrs_json,
    n.version = 'v1', n.updated_at = datetime();


// ========== 5. 导入 L4 实例关系 ==========
LOAD CSV WITH HEADERS FROM 'file:///l4_instance_edges.csv' AS row
MATCH (s:ResourceInstance {node_id: row.source_node_id})
MATCH (t:ResourceInstance {node_id: row.target_node_id})
MERGE (s)-[r:RELATES_TO {edge_id: row.edge_id}]->(t)
SET r.relationship_type = row.relationship_type,
    r.relationship_name = row.relationship_name,
    r.dependency_strength = row.dependency_strength,
    r.is_required = row.is_required,
    r.discovery_method = row.discovery_method,
    r.health_status = row.health_status,
    r.risk_signal = row.risk_signal,
    r.last_verified_at = row.last_verified_at,
    r.attrs_json = row.attrs_json,
    r.version = 'v1', r.updated_at = datetime();


// ========== 6. 常用查询示例 ==========

// 6.1 查看完整应用拓扑（含 L3 Pod/Container/Node）
MATCH path = (app:ResourceInstance {label: 'Application', node_id: 'app:order'})-[*1..6]-(n:ResourceInstance)
RETURN path LIMIT 200;

// 6.2 查看应用所有未关闭巡检发现
MATCH (finding:InspectionFinding {inspection_status: 'failed'})-[:AFFECTS|PROPAGATES_TO*1..4]-(app:ResourceInstance:Application {node_id: 'app:order'})
RETURN finding, app;

// 6.3 查看当前 Firing 告警
MATCH (alert:AlertEvent {lifecycle_status: 'active'})-[r:FIRED_ON]->(resource:ResourceInstance)
WHERE alert.health_status = 'critical'
RETURN alert, resource, r;

// 6.4 查看节点影响范围（爆炸半径）
MATCH path = (node:KubernetesNode {node_id: 'node:cce-prod-01:worker-02'})<-[*1..4]-(affected:ResourceInstance)
WHERE ALL(r IN relationships(path) WHERE r.relationship_type IN [
  'SCHEDULED_ON','CONTAINS','DEPLOYED_AS','BELONGS_TO','RUNS'
])
RETURN node, affected;

// 6.5 查看 Pod 最新指标快照
MATCH (pod:Pod {node_id: 'pod:cce-prod-01:order:order-api-6fd9c8b7c9-abcdf'})
MATCH (snap:MetricSnapshot {resource_id: pod.node_id})
RETURN pod.name, snap.metric_name, snap.current_value, snap.unit, snap.fetched_at
ORDER BY snap.fetched_at DESC;
"""

    filepath = os.path.join(OUTPUT_DIR, "neo4j_import_l3_l4_v1.cypher")
    with open(filepath, "w", encoding="utf-8") as f:
        f.write(cypher)
    print(f"  ✓ neo4j_import_l3_l4_v1.cypher")


def main():
    print("=" * 60)
    print("Generating ALL Mock Data for SRE Inspection Graph")
    print("=" * 60)

    print("\n[1/3] L3 Dynamic Metrics Layer...")
    sys.path.insert(0, SCRIPT_DIR)
    run_module("generate_l3_mock_data")

    print("\n[2/3] L4 Inspection Results Layer...")
    run_module("generate_l4_mock_data")

    print("\n[3/3] Neo4j Import Cypher Script...")
    generate_cypher_import_script()

    print("\n" + "=" * 60)
    print("All mock data generated successfully!")
    print(f"Output directory: {OUTPUT_DIR}")
    print("=" * 60)


if __name__ == "__main__":
    main()
