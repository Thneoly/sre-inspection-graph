#!/usr/bin/env python3
"""
L3 Mock Data Generator — 生成动态观测层模拟数据

输出到 scripts/output/:
  - l3_type_extensions.csv       新增类型节点
  - l3_type_relationship_extensions.csv  新增类型关系
  - l3_instance_nodes.csv         Pod/Container/KubernetesNode 实例
  - l3_instance_edges.csv         实例关系 (CONTAINS, RUNS, SCHEDULED_ON)
  - l3_metric_queries.csv         MetricQuery 定义
  - l3_metric_snapshots.csv       MetricSnapshot 快照值
"""

import csv
import os
from datetime import datetime

OUTPUT_DIR = os.path.join(os.path.dirname(__file__), "output")
os.makedirs(OUTPUT_DIR, exist_ok=True)

NOW = datetime(2026, 6, 15, 10, 0, 0)
NOW_ISO = NOW.isoformat() + "+08:00"

# ============================================================
# 共享常量 — 跨 L2/L3/L4 ID 对齐
# ============================================================

CLUSTER_ID = "cce-prod-01"
NAMESPACE = "order"
DEPLOY_PREFIX = f"{CLUSTER_ID}:{NAMESPACE}:order-api"
POD_PREFIX = f"{CLUSTER_ID}:{NAMESPACE}:order-api"

NODES = [
    {"name": "worker-01", "ip": "10.10.1.21", "cpu": "8", "mem": "32Gi", "status": "Ready"},
    {"name": "worker-02", "ip": "10.10.1.22", "cpu": "8", "mem": "32Gi", "status": "Ready"},
    {"name": "worker-03", "ip": "10.10.1.23", "cpu": "4", "mem": "16Gi", "status": "Ready"},
]

PODS = [
    {"hash": "6fd9c8b7c9-abcde", "node": "worker-01", "ip": "10.244.1.23", "phase": "Running",  "ready": True,  "restart": 1, "cpu": 45.2, "mem": 62.8, "qps": 1250.0, "error_rate": 0.002},
    {"hash": "6fd9c8b7c9-abcdz", "node": "worker-01", "ip": "10.244.1.24", "phase": "Running",  "ready": True,  "restart": 0, "cpu": 32.1, "mem": 55.3, "qps": 1180.0, "error_rate": 0.001},
    {"hash": "6fd9c8b7c9-abcdf", "node": "worker-02", "ip": "10.244.1.25", "phase": "Running",  "ready": True,  "restart": 3, "cpu": 86.5, "mem": 72.3, "qps": 1320.0, "error_rate": 0.072},
    {"hash": "6fd9c8b7c9-abcdg", "node": "worker-02", "ip": "10.244.1.26", "phase": "Running",  "ready": True,  "restart": 0, "cpu": 28.7, "mem": 48.1, "qps": 1010.0, "error_rate": 0.001},
    {"hash": "6fd9c8b7c9-abcdh", "node": "worker-02", "ip": "10.244.1.27", "phase": "Running",  "ready": True,  "restart": 1, "cpu": 51.3, "mem": 66.9, "qps": 1190.0, "error_rate": 0.003},
    {"hash": "6fd9c8b7c9-abcdi", "node": "worker-03", "ip": "10.244.1.28", "phase": "Running",  "ready": True,  "restart": 0, "cpu": 22.5, "mem": 40.2, "qps": 980.0,  "error_rate": 0.001},
    {"hash": "6fd9c8b7c9-abcdj", "node": "worker-03", "ip": "10.244.1.29", "phase": "Pending",  "ready": False, "restart": 0, "cpu": 0,    "mem": 0,    "qps": 0,      "error_rate": 0},
    {"hash": "6fd9c8b7c9-abcdk", "node": "worker-03", "ip": "10.244.1.30", "phase": "Running",  "ready": True,  "restart": 0, "cpu": 35.6, "mem": 52.4, "qps": 1100.0, "error_rate": 0.002},
    {"hash": "6fd9c8b7c9-abcdl", "node": "worker-01", "ip": "10.244.1.31", "phase": "Running",  "ready": True,  "restart": 2, "cpu": 48.9, "mem": 58.7, "qps": 1210.0, "error_rate": 0.004},
]

# ============================================================
# 1. 新增类型节点 (RT-015 ~ RT-019)
# ============================================================

L3_TYPE_NODES = [
    {
        "node_id": "RT-015", "node_name": "Pod", "node_label": "Pod",
        "node_group": "运行态", "abstraction_level": "L4 运行层",
        "scope": "Namespace/Node", "lifecycle_type": "动态对象",
        "unique_key": "cluster_id + namespace + pod_name",
        "key_properties": "pod_ip, host_ip, node_name, phase, ready, restart_count, owner_kind, owner_name",
        "inspection_focus": "Pod phase, restart count, resource usage, scheduling status, health probes",
        "health_fields": "phase, ready, restart_count, health_status, risk_level",
        "required_relation_summary": "SCHEDULED_ON KubernetesNode; RUNS Container; BELONGS_TO Namespace; CONTROLLED_BY Deployment",
        "import_label": ":ResourceType:Pod",
    },
    {
        "node_id": "RT-016", "node_name": "容器", "node_label": "Container",
        "node_group": "运行态", "abstraction_level": "L4 运行层",
        "scope": "Pod", "lifecycle_type": "动态对象",
        "unique_key": "cluster_id + namespace + pod_name + container_name",
        "key_properties": "container_name, image, image_digest, cpu_request, memory_request, cpu_limit, memory_limit",
        "inspection_focus": "Image pull status, OOM kills, CPU/memory throttling, readiness probe",
        "health_fields": "ready, restart_count, cpu_usage_pct, memory_usage_pct",
        "required_relation_summary": "RUNS_IN Pod; USES ContainerImage",
        "import_label": ":ResourceType:Container",
    },
    {
        "node_id": "RT-017", "node_name": "Kubernetes节点", "node_label": "KubernetesNode",
        "node_group": "平台位置", "abstraction_level": "L3 平台层",
        "scope": "Cluster", "lifecycle_type": "半稳定对象",
        "unique_key": "cluster_id + node_name",
        "key_properties": "node_ip, instance_type, cpu_capacity, memory_capacity, kernel_version, kubelet_version",
        "inspection_focus": "Node readiness, resource pressure, kernel vulnerabilities, disk pressure",
        "health_fields": "node_status, cpu_pressure, memory_pressure, disk_pressure",
        "required_relation_summary": "BELONGS_TO KubernetesCluster; SCHEDULES Pod",
        "import_label": ":ResourceType:KubernetesNode",
    },
    {
        "node_id": "RT-018", "node_name": "指标查询模板", "node_label": "MetricQuery",
        "node_group": "可观测", "abstraction_level": "L5 观测层",
        "scope": "Global/ResourceType", "lifecycle_type": "配置对象",
        "unique_key": "query_id",
        "key_properties": "query_id, metric_name, promql_template, target_resource_type, datasource_uid",
        "inspection_focus": "PromQL validity, datasource availability, threshold configuration",
        "health_fields": "enabled_status, datasource_status",
        "required_relation_summary": "HAS_METRIC ResourceType; SNAPSHOTS_TO MetricSnapshot",
        "import_label": ":ResourceType:MetricQuery",
    },
    {
        "node_id": "RT-019", "node_name": "指标快照", "node_label": "MetricSnapshot",
        "node_group": "可观测", "abstraction_level": "L5 观测层",
        "scope": "ResourceInstance", "lifecycle_type": "动态对象（latest-N）",
        "unique_key": "resource_id + metric_name + fetched_at",
        "key_properties": "metric_name, current_value, unit, fetched_at, is_stale, threshold_breached",
        "inspection_focus": "Value thresholds, staleness, trend deviation",
        "health_fields": "is_stale, warning_breached, critical_breached",
        "required_relation_summary": "MEASURES ResourceInstance; SNAPSHOTTED_BY MetricQuery",
        "import_label": ":ResourceType:MetricSnapshot",
    },
]

# ============================================================
# 2. 新增类型关系 (REL-035 ~ REL-041)
# ============================================================

L3_TYPE_EDGES = [
    {
        "edge_id": "REL-035", "source_node_id": "RT-006", "source_node_name": "Deployment",
        "source_label": "Deployment", "relationship_type": "CONTAINS", "relationship_name": "包含",
        "target_node_id": "RT-015", "target_node_name": "Pod", "target_label": "Pod",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是", "inspection_purpose": "Deployment 管理 Pod 副本（通过 ReplicaSet）",
        "inspection_check_item": "Pod 数量与期望副本数一致性；CrashLoopBackOff；Pending 超时",
        "risk_signal": "Pod 数量不足；频繁重启；Pending 超过 N 分钟",
        "impact_direction": "Deployment -> Pod -> Container",
        "alert_aggregation": "Pod 告警向 Deployment 归并",
        "discovery_method": "Kubernetes API 标签匹配",
        "graph_view": "应用拓扑视图",
    },
    {
        "edge_id": "REL-036", "source_node_id": "RT-015", "source_node_name": "Pod",
        "source_label": "Pod", "relationship_type": "RUNS", "relationship_name": "运行",
        "target_node_id": "RT-016", "target_node_name": "容器", "target_label": "Container",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是", "inspection_purpose": "Pod 内含一个或多个容器",
        "inspection_check_item": "容器是否 Ready；镜像拉取是否成功；OOM Kill 次数",
        "risk_signal": "ContainerCreating 超时；ImagePullBackOff；OOMKilled",
        "impact_direction": "Pod -> Container",
        "alert_aggregation": "容器异常向 Pod 归并",
        "discovery_method": "Kubernetes API",
        "graph_view": "应用拓扑视图",
    },
    {
        "edge_id": "REL-037", "source_node_id": "RT-015", "source_node_name": "Pod",
        "source_label": "Pod", "relationship_type": "SCHEDULED_ON", "relationship_name": "调度在",
        "target_node_id": "RT-017", "target_node_name": "Kubernetes节点", "target_label": "KubernetesNode",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是", "inspection_purpose": "Pod 被调度到特定节点——节点故障爆炸半径分析核心关系",
        "inspection_check_item": "节点资源是否充足；Affinity/Anti-affinity 冲突",
        "risk_signal": "节点不可用导致所有 Pod 受影响；节点磁盘压力导致 Pod Eviction",
        "impact_direction": "Node -> Pod -> Deployment -> Component -> App",
        "alert_aggregation": "节点告警向下关联到 Pod",
        "discovery_method": "Kubernetes API",
        "graph_view": "节点影响视图",
    },
    {
        "edge_id": "REL-038", "source_node_id": "RT-017", "source_node_name": "Kubernetes节点",
        "source_label": "KubernetesNode", "relationship_type": "BELONGS_TO", "relationship_name": "属于",
        "target_node_id": "RT-004", "target_node_name": "Kubernetes集群", "target_label": "KubernetesCluster",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是", "inspection_purpose": "节点是集群成员——集群级巡检入口",
        "inspection_check_item": "节点状态 Ready/NotReady；节点组/可用区分布",
        "risk_signal": "大量节点 NotReady；节点分布不均",
        "impact_direction": "Cluster -> Node -> Pod",
        "alert_aggregation": "节点告警按集群聚合",
        "discovery_method": "Kubernetes API/集群注册表",
        "graph_view": "平台位置视图",
    },
    {
        "edge_id": "REL-039", "source_node_id": "RT-015", "source_node_name": "Pod",
        "source_label": "Pod", "relationship_type": "HAS_METRIC", "relationship_name": "有指标查询",
        "target_node_id": "RT-018", "target_node_name": "指标查询模板", "target_label": "MetricQuery",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "半自动",
        "impact_analysis": "否", "inspection_purpose": "关联指标查询模板，前端按需查询 Prometheus",
        "inspection_check_item": "指标查询是否有效；数据源是否可达",
        "risk_signal": "指标查询返回空；数据源不可用",
        "impact_direction": "",
        "alert_aggregation": "",
        "discovery_method": "手动配置 + Prometheus 服务发现",
        "graph_view": "指标覆盖视图",
    },
    {
        "edge_id": "REL-040", "source_node_id": "RT-018", "source_node_name": "指标查询模板",
        "source_label": "MetricQuery", "relationship_type": "SNAPSHOTS_TO", "relationship_name": "快照到",
        "target_node_id": "RT-019", "target_node_name": "指标快照", "target_label": "MetricSnapshot",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "自动",
        "impact_analysis": "否", "inspection_purpose": "查询定期产生快照值，存入 Neo4j 供快速展示",
        "inspection_check_item": "快照是否在 TTL 内；快照值趋势是否偏离基线",
        "risk_signal": "快照过期；值超过阈值",
        "impact_direction": "",
        "alert_aggregation": "",
        "discovery_method": "指标采集器定时同步",
        "graph_view": "指标趋势视图",
    },
    {
        "edge_id": "REL-041", "source_node_id": "RT-019", "source_node_name": "指标快照",
        "source_label": "MetricSnapshot", "relationship_type": "MEASURES", "relationship_name": "测量",
        "target_node_id": "RT-015", "target_node_name": "Pod", "target_label": "Pod",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "自动",
        "impact_analysis": "否", "inspection_purpose": "快照关联到具体资源实例",
        "inspection_check_item": "快照值与资源节点匹配",
        "risk_signal": "快照关联到不存在的资源",
        "impact_direction": "",
        "alert_aggregation": "",
        "discovery_method": "指标采集器标签匹配",
        "graph_view": "指标趋势视图",
    },
]

# ============================================================
# 3. MetricQuery 定义
# ============================================================

METRIC_QUERIES = [
    {"query_id": "mq-cpu-usage",      "metric_name": "cpu_usage",      "target_resource_type": "Pod",            "promql_template": 'sum(rate(container_cpu_usage_seconds_total{namespace="{{namespace}}",pod="{{pod}}"}[5m])) * 100', "datasource_uid": "prometheus-prod", "unit": "percent",      "warning_threshold": "80", "critical_threshold": "95"},
    {"query_id": "mq-memory-usage",   "metric_name": "memory_usage",   "target_resource_type": "Pod",            "promql_template": 'container_memory_working_set_bytes{namespace="{{namespace}}",pod="{{pod}}"}',                "datasource_uid": "prometheus-prod", "unit": "bytes",        "warning_threshold": "80", "critical_threshold": "95"},
    {"query_id": "mq-qps",            "metric_name": "qps",            "target_resource_type": "Pod",            "promql_template": 'sum(rate(http_requests_total{namespace="{{namespace}}",pod="{{pod}}"}[5m]))',             "datasource_uid": "prometheus-prod", "unit": "requests/s",   "warning_threshold": "",   "critical_threshold": ""},
    {"query_id": "mq-error-rate",     "metric_name": "error_rate",     "target_resource_type": "Pod",            "promql_template": 'sum(rate(http_requests_total{namespace="{{namespace}}",pod="{{pod}}",status=~"5.."}[5m])) / sum(rate(http_requests_total{namespace="{{namespace}}",pod="{{pod}}"}[5m]))', "datasource_uid": "prometheus-prod", "unit": "fraction", "warning_threshold": "0.01", "critical_threshold": "0.05"},
    {"query_id": "mq-restart-count",  "metric_name": "restart_count",  "target_resource_type": "Pod",            "promql_template": 'kube_pod_container_status_restarts_total{namespace="{{namespace}}",pod="{{pod}}"}',        "datasource_uid": "prometheus-prod", "unit": "count",        "warning_threshold": "3",  "critical_threshold": "10"},
    {"query_id": "mq-node-cpu",       "metric_name": "node_cpu_usage", "target_resource_type": "KubernetesNode", "promql_template": '100 - (avg(rate(node_cpu_seconds_total{mode="idle",node="{{node_name}}"}[5m])) * 100)', "datasource_uid": "prometheus-prod", "unit": "percent",  "warning_threshold": "80", "critical_threshold": "95"},
    {"query_id": "mq-node-memory",    "metric_name": "node_memory_usage", "target_resource_type": "KubernetesNode", "promql_template": '(1 - node_memory_MemAvailable_bytes{node="{{node_name}}"} / node_memory_MemTotal_bytes{node="{{node_name}}"}) * 100', "datasource_uid": "prometheus-prod", "unit": "percent", "warning_threshold": "80", "critical_threshold": "95"},
]

# ============================================================
# 4. 生成实例节点
# ============================================================

def _health(phase, ready, restart):
    if not ready or phase != "Running":
        return "critical", "high"
    if restart >= 3:
        return "critical", "high"
    if restart >= 1:
        return "warning", "medium"
    return "normal", "low"

def generate_l3_instance_nodes():
    """生成 Pod(9), Container(9), KubernetesNode(3) 实例"""
    rows = []

    # --- KubernetesNodes ---
    for n in NODES:
        cpu_pct = round(40 + hash(n["name"]) % 30, 1)
        mem_pct = round(50 + hash(n["name"] + "m") % 25, 1)
        disk_pct = round(30 + hash(n["name"] + "d") % 40, 1)
        rows.append({
            "node_id": f"node:{CLUSTER_ID}:{n['name']}",
            "label": "KubernetesNode",
            "name": n["name"],
            "unique_key": f"{CLUSTER_ID}/{n['name']}",
            "env_code": "prod",
            "app_code": "",
            "component_code": "",
            "cluster_id": CLUSTER_ID,
            "namespace": "",
            "owner_team": "平台团队",
            "lifecycle_status": "active",
            "health_status": "normal",
            "risk_level": "low",
            "inspection_status": "passed",
            "last_inspected_at": NOW_ISO,
            "source_system": "Kubernetes",
            "source_ref": f"nodes/{n['name']}",
            "attrs_json": f'{{"node_ip":"{n["ip"]}","instance_type":"c6.2xlarge","cpu_capacity":"{n["cpu"]}","memory_capacity":"{n["mem"]}","pod_capacity":110,"kernel_version":"5.15.0-1025-aws","kubelet_version":"v1.29.3","node_status":"{n["status"]}","cpu_usage_percent":{cpu_pct},"memory_usage_percent":{mem_pct},"disk_usage_percent":{disk_pct},"conditions":[{{"type":"MemoryPressure","status":"False"}},{{"type":"DiskPressure","status":"False"}},{{"type":"PIDPressure","status":"False"}}]}}',
        })

    # --- Pods ---
    for p in PODS:
        h, r = _health(p["phase"], p["ready"], p["restart"])
        rows.append({
            "node_id": f"pod:{POD_PREFIX}-{p['hash']}",
            "label": "Pod",
            "name": f"order-api-{p['hash']}",
            "unique_key": f"{CLUSTER_ID}/{NAMESPACE}/order-api-{p['hash']}",
            "env_code": "prod",
            "app_code": "order",
            "component_code": "order-api",
            "cluster_id": CLUSTER_ID,
            "namespace": NAMESPACE,
            "owner_team": "订单团队",
            "lifecycle_status": "active",
            "health_status": h,
            "risk_level": r,
            "inspection_status": "passed" if h == "normal" else "failed",
            "last_inspected_at": NOW_ISO,
            "source_system": "Kubernetes",
            "source_ref": f"pods/{NAMESPACE}/order-api-{p['hash']}",
            "attrs_json": f'{{"pod_ip":"{p["ip"]}","host_ip":"{next(n["ip"] for n in NODES if n["name"] == p["node"])}","node_name":"{p["node"]}","phase":"{p["phase"]}","ready":{str(p["ready"]).lower()},"restart_count":{p["restart"]},"owner_kind":"ReplicaSet","owner_name":"order-api-6fd9c8b7c9","cpu_usage_percent":{p["cpu"]},"memory_usage_percent":{p["mem"]},"qps":{p["qps"]},"error_rate":{p["error_rate"]},"metric_source":"Prometheus","log_source":"Loki","uid":"{p['hash'].replace("-","")}"}}',
        })

    # --- Containers (one per pod) ---
    for p in PODS:
        rows.append({
            "node_id": f"container:{POD_PREFIX}-{p['hash']}:order-api",
            "label": "Container",
            "name": "order-api",
            "unique_key": f"{CLUSTER_ID}/{NAMESPACE}/order-api-{p['hash']}/order-api",
            "env_code": "prod",
            "app_code": "order",
            "component_code": "order-api",
            "cluster_id": CLUSTER_ID,
            "namespace": NAMESPACE,
            "owner_team": "订单团队",
            "lifecycle_status": "active",
            "health_status": "normal" if p["ready"] else "critical",
            "risk_level": "low" if p["ready"] else "high",
            "inspection_status": "passed" if p["ready"] else "failed",
            "last_inspected_at": NOW_ISO,
            "source_system": "Kubernetes",
            "source_ref": f"containers/{NAMESPACE}/order-api-{p['hash']}/order-api",
            "attrs_json": f'{{"image":"registry.example.com/order/order-api:v1.2.3","image_digest":"sha256:abc123def456","cpu_request":"500m","memory_request":"512Mi","cpu_limit":"2000m","memory_limit":"2048Mi","cpu_usage_percent":{p["cpu"]},"memory_usage_percent":{p["mem"]},"restart_count":{p["restart"]},"ready":{str(p["ready"]).lower()},"ports":"[8080]"}}',
        })

    return rows


def generate_l3_instance_edges():
    """生成 L3 实例关系"""
    rows = []
    eid = 100

    # Deployment CONTAINS Pod
    for p in PODS:
        eid += 1
        pod_id = f"pod:{POD_PREFIX}-{p['hash']}"
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": f"deploy:{DEPLOY_PREFIX}",
            "relationship_type": "CONTAINS",
            "target_node_id": pod_id,
            "relationship_name": "包含",
            "dependency_strength": "强",
            "is_required": "是",
            "discovery_method": "Kubernetes API",
            "health_status": "normal" if p["ready"] else "warning",
            "risk_signal": "" if p["ready"] else f"Pod {p['phase']}, 重启{p['restart']}次",
            "last_verified_at": NOW_ISO,
            "attrs_json": "{}",
        })

    # Pod RUNS Container & SCHEDULED_ON Node
    for p in PODS:
        pod_id = f"pod:{POD_PREFIX}-{p['hash']}"
        container_id = f"container:{POD_PREFIX}-{p['hash']}:order-api"
        node_id = f"node:{CLUSTER_ID}:{p['node']}"

        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": pod_id,
            "relationship_type": "RUNS",
            "target_node_id": container_id,
            "relationship_name": "运行",
            "dependency_strength": "强",
            "is_required": "是",
            "discovery_method": "Kubernetes API",
            "health_status": "normal" if p["ready"] else "warning",
            "risk_signal": "",
            "last_verified_at": NOW_ISO,
            "attrs_json": "{}",
        })

        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": pod_id,
            "relationship_type": "SCHEDULED_ON",
            "target_node_id": node_id,
            "relationship_name": "调度在",
            "dependency_strength": "强",
            "is_required": "是",
            "discovery_method": "Kubernetes API",
            "health_status": "normal",
            "risk_signal": "",
            "last_verified_at": NOW_ISO,
            "attrs_json": "{}",
        })

    # KubernetesNode BELONGS_TO KubernetesCluster
    for n in NODES:
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": f"node:{CLUSTER_ID}:{n['name']}",
            "relationship_type": "BELONGS_TO",
            "target_node_id": f"cluster:{CLUSTER_ID}",
            "relationship_name": "属于",
            "dependency_strength": "强",
            "is_required": "是",
            "discovery_method": "Kubernetes API",
            "health_status": "normal",
            "risk_signal": "",
            "last_verified_at": NOW_ISO,
            "attrs_json": "{}",
        })

    return rows


def generate_metric_queries():
    """MetricQuery 实例"""
    rows = []
    for mq in METRIC_QUERIES:
        rows.append({
            "query_id": mq["query_id"],
            "metric_name": mq["metric_name"],
            "target_resource_type": mq["target_resource_type"],
            "promql_template": mq["promql_template"],
            "datasource_uid": mq["datasource_uid"],
            "unit": mq["unit"],
            "warning_threshold": mq["warning_threshold"],
            "critical_threshold": mq["critical_threshold"],
            "enabled_status": "enabled",
            "datasource_status": "connected",
        })
    return rows


def generate_metric_snapshots():
    """生成每个 Pod 和 Node 的指标快照"""
    rows = []

    for p in PODS:
        pod_id = f"pod:{POD_PREFIX}-{p['hash']}"
        snapshots = []

        # CPU
        w, c = _threshold_status(p["cpu"], 80, 95)
        snapshots.append({
            "snapshot_id": f"snap_{pod_id}_cpu_{NOW.strftime('%Y%m%d%H%M%S')}",
            "resource_id": pod_id,
            "metric_name": "cpu_usage", "metric_query_id": "mq-cpu-usage",
            "current_value": p["cpu"], "unit": "percent",
            "fetched_at": NOW_ISO, "ttl_seconds": 300,
            "is_stale": "false", "warning_breached": str(w).lower(), "critical_breached": str(c).lower(),
        })
        # Memory
        w, c = _threshold_status(p["mem"], 80, 95)
        snapshots.append({
            "snapshot_id": f"snap_{pod_id}_memory_{NOW.strftime('%Y%m%d%H%M%S')}",
            "resource_id": pod_id,
            "metric_name": "memory_usage", "metric_query_id": "mq-memory-usage",
            "current_value": p["mem"], "unit": "percent",
            "fetched_at": NOW_ISO, "ttl_seconds": 300,
            "is_stale": "false", "warning_breached": str(w).lower(), "critical_breached": str(c).lower(),
        })
        # QPS
        snapshots.append({
            "snapshot_id": f"snap_{pod_id}_qps_{NOW.strftime('%Y%m%d%H%M%S')}",
            "resource_id": pod_id,
            "metric_name": "qps", "metric_query_id": "mq-qps",
            "current_value": p["qps"], "unit": "requests/s",
            "fetched_at": NOW_ISO, "ttl_seconds": 300,
            "is_stale": "false", "warning_breached": "false", "critical_breached": "false",
        })
        # Error Rate
        w, c = _threshold_status(p["error_rate"], 0.01, 0.05)
        snapshots.append({
            "snapshot_id": f"snap_{pod_id}_error_rate_{NOW.strftime('%Y%m%d%H%M%S')}",
            "resource_id": pod_id,
            "metric_name": "error_rate", "metric_query_id": "mq-error-rate",
            "current_value": p["error_rate"], "unit": "fraction",
            "fetched_at": NOW_ISO, "ttl_seconds": 300,
            "is_stale": "false", "warning_breached": str(w).lower(), "critical_breached": str(c).lower(),
        })
        # Restart count
        w, c = _threshold_status(p["restart"], 3, 10)
        snapshots.append({
            "snapshot_id": f"snap_{pod_id}_restart_{NOW.strftime('%Y%m%d%H%M%S')}",
            "resource_id": pod_id,
            "metric_name": "restart_count", "metric_query_id": "mq-restart-count",
            "current_value": p["restart"], "unit": "count",
            "fetched_at": NOW_ISO, "ttl_seconds": 600,
            "is_stale": "false", "warning_breached": str(w).lower(), "critical_breached": str(c).lower(),
        })
        rows.extend(snapshots)

    # Node metrics
    for n in NODES:
        node_id = f"node:{CLUSTER_ID}:{n['name']}"
        cpu_pct = round(40 + hash(n["name"]) % 30, 1)
        mem_pct = round(50 + hash(n["name"] + "m") % 25, 1)

        w, c = _threshold_status(cpu_pct, 80, 95)
        rows.append({
            "snapshot_id": f"snap_{node_id}_node_cpu_{NOW.strftime('%Y%m%d%H%M%S')}",
            "resource_id": node_id,
            "metric_name": "node_cpu_usage", "metric_query_id": "mq-node-cpu",
            "current_value": cpu_pct, "unit": "percent",
            "fetched_at": NOW_ISO, "ttl_seconds": 300,
            "is_stale": "false", "warning_breached": str(w).lower(), "critical_breached": str(c).lower(),
        })
        w, c = _threshold_status(mem_pct, 80, 95)
        rows.append({
            "snapshot_id": f"snap_{node_id}_node_memory_{NOW.strftime('%Y%m%d%H%M%S')}",
            "resource_id": node_id,
            "metric_name": "node_memory_usage", "metric_query_id": "mq-node-memory",
            "current_value": mem_pct, "unit": "percent",
            "fetched_at": NOW_ISO, "ttl_seconds": 300,
            "is_stale": "false", "warning_breached": str(w).lower(), "critical_breached": str(c).lower(),
        })

    return rows


def _threshold_status(value: float, warning: float, critical: float) -> tuple[bool, bool]:
    """判断是否超过阈值"""
    if value >= critical:
        return True, True
    if value >= warning:
        return True, False
    return False, False


# ============================================================
# 5. CSV 写入
# ============================================================

CSV_COLUMNS_L3_TYPE_NODES = [
    "node_id", "node_name", "node_label", "node_group", "abstraction_level",
    "scope", "lifecycle_type", "unique_key", "key_properties",
    "inspection_focus", "health_fields", "required_relation_summary", "import_label",
]

CSV_COLUMNS_L3_TYPE_EDGES = [
    "edge_id", "source_node_id", "source_node_name", "source_label",
    "relationship_type", "relationship_name",
    "target_node_id", "target_node_name", "target_label",
    "dependency_strength", "is_required", "auto_discovery",
    "impact_analysis", "inspection_purpose", "inspection_check_item",
    "risk_signal", "impact_direction", "alert_aggregation",
    "discovery_method", "graph_view",
]

INSTANCE_NODE_COLUMNS = [
    "node_id", "label", "name", "unique_key",
    "env_code", "app_code", "component_code", "cluster_id", "namespace",
    "owner_team", "lifecycle_status", "health_status", "risk_level",
    "inspection_status", "last_inspected_at", "source_system", "source_ref", "attrs_json",
]

INSTANCE_EDGE_COLUMNS = [
    "edge_id", "source_node_id", "relationship_type", "target_node_id",
    "relationship_name", "dependency_strength", "is_required",
    "discovery_method", "health_status", "risk_signal",
    "last_verified_at", "attrs_json",
]

MQ_COLUMNS = [
    "query_id", "metric_name", "target_resource_type", "promql_template",
    "datasource_uid", "unit", "warning_threshold", "critical_threshold",
    "enabled_status", "datasource_status",
]

SNAPSHOT_COLUMNS = [
    "snapshot_id", "resource_id", "metric_name", "metric_query_id",
    "current_value", "unit", "fetched_at", "ttl_seconds",
    "is_stale", "warning_breached", "critical_breached",
]


def write_csv(filename: str, columns: list, rows: list):
    filepath = os.path.join(OUTPUT_DIR, filename)
    with open(filepath, "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=columns, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    print(f"  ✓ {filename} ({len(rows)} rows)")


def main():
    print("Generating L3 Mock Data...")

    write_csv("l3_type_extensions.csv", CSV_COLUMNS_L3_TYPE_NODES, L3_TYPE_NODES)
    write_csv("l3_type_relationship_extensions.csv", CSV_COLUMNS_L3_TYPE_EDGES, L3_TYPE_EDGES)

    nodes = generate_l3_instance_nodes()
    write_csv("l3_instance_nodes.csv", INSTANCE_NODE_COLUMNS, nodes)

    edges = generate_l3_instance_edges()
    write_csv("l3_instance_edges.csv", INSTANCE_EDGE_COLUMNS, edges)

    mq = generate_metric_queries()
    write_csv("l3_metric_queries.csv", MQ_COLUMNS, mq)

    ms = generate_metric_snapshots()
    write_csv("l3_metric_snapshots.csv", SNAPSHOT_COLUMNS, ms)

    total = len(nodes) + len(edges) + len(mq) + len(ms)
    print(f"\nTotal L3 records: {total}")
    print("Done.")


if __name__ == "__main__":
    main()
