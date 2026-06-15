// 云原生应用巡检图谱 v1 - Neo4j 导入示例
// 使用方式：将 CSV 放入 Neo4j import 目录后，在 Browser / cypher-shell 中执行。
// 本脚本包含两套关系导入方式：
// A. 无 APOC：使用统一关系 :RELATES_TO，并把真实关系类型写入 relationship_type 属性。
// B. 有 APOC：使用 apoc.create.relationship 创建动态关系类型，如 :CONTAINS、:USES、:MONITORS。
// 二选一执行即可，建议先使用 A 验证导入流程，再使用 B 生成更直观的关系类型图。

// ========== 0. 约束 ==========
CREATE CONSTRAINT resource_type_node_id IF NOT EXISTS
FOR (n:ResourceType) REQUIRE n.node_id IS UNIQUE;

CREATE CONSTRAINT resource_instance_node_id IF NOT EXISTS
FOR (n:ResourceInstance) REQUIRE n.node_id IS UNIQUE;


// ========== 1. 导入类型图节点：14 类主数据对象 ==========
LOAD CSV WITH HEADERS FROM 'file:///cloud_native_graph_type_nodes_v1.csv' AS row
MERGE (n:ResourceType {node_id: row.node_id})
SET
  n.name = row.node_name,
  n.label = row.node_label,
  n.node_group = row.node_group,
  n.abstraction_level = row.abstraction_level,
  n.scope = row.scope,
  n.lifecycle_type = row.lifecycle_type,
  n.unique_key = row.unique_key,
  n.key_properties = row.key_properties,
  n.inspection_focus = row.inspection_focus,
  n.health_fields = row.health_fields,
  n.required_relation_summary = row.required_relation_summary,
  n.import_label = row.import_label,
  n.version = 'v1',
  n.updated_at = datetime();


// ========== 2A. 导入类型图关系：无 APOC 兼容版 ==========
LOAD CSV WITH HEADERS FROM 'file:///cloud_native_graph_type_edges_v1.csv' AS row
MATCH (s:ResourceType {node_id: row.source_node_id})
MATCH (t:ResourceType {node_id: row.target_node_id})
MERGE (s)-[r:RELATES_TO {edge_id: row.edge_id}]->(t)
SET
  r.relationship_type = row.relationship_type,
  r.relationship_name = row.relationship_name,
  r.dependency_strength = row.dependency_strength,
  r.is_required = row.is_required,
  r.auto_discovery = row.auto_discovery,
  r.impact_analysis = row.impact_analysis,
  r.inspection_purpose = row.inspection_purpose,
  r.inspection_check_item = row.inspection_check_item,
  r.risk_signal = row.risk_signal,
  r.impact_direction = row.impact_direction,
  r.alert_aggregation = row.alert_aggregation,
  r.discovery_method = row.discovery_method,
  r.graph_view = row.graph_view,
  r.remark = row.remark,
  r.version = 'v1',
  r.updated_at = datetime();


// ========== 2B. 导入类型图关系：APOC 动态关系类型版 ==========
// 如需使用，请确认已安装 APOC，并注释掉 2A，避免重复关系。
// LOAD CSV WITH HEADERS FROM 'file:///cloud_native_graph_type_edges_v1.csv' AS row
// MATCH (s:ResourceType {node_id: row.source_node_id})
// MATCH (t:ResourceType {node_id: row.target_node_id})
// CALL apoc.create.relationship(
//   s,
//   row.relationship_type,
//   {
//     edge_id: row.edge_id,
//     relationship_name: row.relationship_name,
//     dependency_strength: row.dependency_strength,
//     is_required: row.is_required,
//     auto_discovery: row.auto_discovery,
//     impact_analysis: row.impact_analysis,
//     inspection_purpose: row.inspection_purpose,
//     inspection_check_item: row.inspection_check_item,
//     risk_signal: row.risk_signal,
//     impact_direction: row.impact_direction,
//     alert_aggregation: row.alert_aggregation,
//     discovery_method: row.discovery_method,
//     graph_view: row.graph_view,
//     remark: row.remark,
//     version: 'v1',
//     updated_at: datetime()
//   },
//   t
// ) YIELD rel
// RETURN count(rel);


// ========== 3. 导入实例图节点模板数据 ==========
LOAD CSV WITH HEADERS FROM 'file:///cloud_native_graph_instance_nodes_template_v1.csv' AS row
MERGE (n:ResourceInstance {node_id: row.node_id})
SET
  n.label = row.node_label,
  n.name = row.node_name,
  n.unique_key = row.unique_key,
  n.env_code = row.env_code,
  n.app_code = row.app_code,
  n.component_code = row.component_code,
  n.cluster_id = row.cluster_id,
  n.namespace = row.namespace,
  n.owner_team = row.owner_team,
  n.lifecycle_status = row.lifecycle_status,
  n.health_status = row.health_status,
  n.risk_level = row.risk_level,
  n.inspection_status = row.inspection_status,
  n.last_inspected_at = row.last_inspected_at,
  n.source_system = row.source_system,
  n.source_ref = row.source_ref,
  n.attrs_json = row.attrs_json,
  n.version = 'v1',
  n.updated_at = datetime();


// ========== 4A. 导入实例图关系：无 APOC 兼容版 ==========
LOAD CSV WITH HEADERS FROM 'file:///cloud_native_graph_instance_edges_template_v1.csv' AS row
MATCH (s:ResourceInstance {node_id: row.source_node_id})
MATCH (t:ResourceInstance {node_id: row.target_node_id})
MERGE (s)-[r:RELATES_TO {edge_id: row.edge_id}]->(t)
SET
  r.relationship_type = row.relationship_type,
  r.relationship_name = row.relationship_name,
  r.dependency_strength = row.dependency_strength,
  r.is_required = row.is_required,
  r.discovery_method = row.discovery_method,
  r.health_status = row.health_status,
  r.risk_signal = row.risk_signal,
  r.last_verified_at = row.last_verified_at,
  r.attrs_json = row.attrs_json,
  r.version = 'v1',
  r.updated_at = datetime();


// ========== 4B. 导入实例图关系：APOC 动态关系类型版 ==========
// 如需使用，请确认已安装 APOC，并注释掉 4A。
// LOAD CSV WITH HEADERS FROM 'file:///cloud_native_graph_instance_edges_template_v1.csv' AS row
// MATCH (s:ResourceInstance {node_id: row.source_node_id})
// MATCH (t:ResourceInstance {node_id: row.target_node_id})
// CALL apoc.create.relationship(
//   s,
//   row.relationship_type,
//   {
//     edge_id: row.edge_id,
//     relationship_name: row.relationship_name,
//     dependency_strength: row.dependency_strength,
//     is_required: row.is_required,
//     discovery_method: row.discovery_method,
//     health_status: row.health_status,
//     risk_signal: row.risk_signal,
//     last_verified_at: row.last_verified_at,
//     attrs_json: row.attrs_json,
//     version: 'v1',
//     updated_at: datetime()
//   },
//   t
// ) YIELD rel
// RETURN count(rel);


// ========== 5. 常用查询示例 ==========

// 5.1 查看类型图全貌
MATCH p=(n:ResourceType)-[r:RELATES_TO]->(m:ResourceType)
RETURN p;

// 5.2 查看某个应用实例的巡检拓扑
MATCH p=(app:ResourceInstance {node_id:'app:order'})-[*1..4]-(n:ResourceInstance)
RETURN p;

// 5.3 查找高风险节点及其上游影响
MATCH p=(risk:ResourceInstance {risk_level:'medium'})-[*1..4]-(n:ResourceInstance)
RETURN p;

// 5.4 查找没有告警规则覆盖的应用组件
MATCH (c:ResourceInstance {label:'ApplicationComponent'})
WHERE NOT EXISTS {
  MATCH (:ResourceInstance {label:'AlertRule'})-[:RELATES_TO]->(c)
}
RETURN c.node_id, c.name, c.app_code;

// 5.5 查找即将过期或高风险 Secret 影响哪些应用
MATCH p=(secret:ResourceInstance {label:'Secret'})<-[:RELATES_TO*1..4]-(n:ResourceInstance)
WHERE secret.risk_level IN ['medium','high','critical']
RETURN p;
