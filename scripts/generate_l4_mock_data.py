#!/usr/bin/env python3
"""
L4 Mock Data Generator — 生成巡检结果层模拟数据

输出到 scripts/output/:
  - l4_type_extensions.csv              新增类型节点
  - l4_type_relationship_extensions.csv  新增类型关系
  - l4_instance_nodes.csv                InspectionRun/Rule/Finding/AlertEvent 实例
  - l4_instance_edges.csv                巡检和告警关系
"""

import csv
import os
from datetime import datetime, timedelta

OUTPUT_DIR = os.path.join(os.path.dirname(__file__), "output")
os.makedirs(OUTPUT_DIR, exist_ok=True)

NOW = datetime(2026, 6, 15, 10, 0, 0)
NOW_ISO = NOW.isoformat() + "+08:00"

# ============================================================
# 共享常量
# ============================================================

CLUSTER_ID = "cce-prod-01"
NAMESPACE = "order"
POD_PREFIX = f"{CLUSTER_ID}:{NAMESPACE}:order-api"

# ============================================================
# 1. 新增类型节点 (RT-020 ~ RT-023)
# ============================================================

L4_TYPE_NODES = [
    {
        "node_id": "RT-020", "node_name": "巡检运行", "node_label": "InspectionRun",
        "node_group": "巡检", "abstraction_level": "L6 巡检层",
        "scope": "Application/Cluster", "lifecycle_type": "记录对象",
        "unique_key": "run_id",
        "key_properties": "run_id, run_name, run_type, started_at, completed_at, overall_status",
        "inspection_focus": "巡检执行状态、覆盖率、执行时长、通过率",
        "health_fields": "overall_status, pass_rate, execution_duration",
        "required_relation_summary": "GENERATED InspectionFinding; EXECUTES InspectionRule",
        "import_label": ":ResourceType:InspectionRun",
    },
    {
        "node_id": "RT-021", "node_name": "巡检规则", "node_label": "InspectionRule",
        "node_group": "巡检", "abstraction_level": "L6 巡检层",
        "scope": "Global/ResourceType", "lifecycle_type": "配置对象",
        "unique_key": "rule_id",
        "key_properties": "rule_id, rule_name, rule_category, severity, applies_to_resource_type",
        "inspection_focus": "规则覆盖完整性、规则有效性、阈值合理性",
        "health_fields": "enabled_status, last_executed_at, hit_rate",
        "required_relation_summary": "APPLIES_TO ResourceType; GENERATES InspectionFinding",
        "import_label": ":ResourceType:InspectionRule",
    },
    {
        "node_id": "RT-022", "node_name": "巡检发现", "node_label": "InspectionFinding",
        "node_group": "巡检", "abstraction_level": "L6 巡检层",
        "scope": "ResourceInstance", "lifecycle_type": "记录对象",
        "unique_key": "finding_id",
        "key_properties": "finding_id, severity, status, affected_resource_id, detected_at",
        "inspection_focus": "发现数量、严重程度分布、修复时效、误报率",
        "health_fields": "status, severity, time_to_resolve",
        "required_relation_summary": "FOUND_IN InspectionRun; VIOLATES InspectionRule; AFFECTS ResourceInstance",
        "import_label": ":ResourceType:InspectionFinding",
    },
    {
        "node_id": "RT-023", "node_name": "告警事件", "node_label": "AlertEvent",
        "node_group": "可观测", "abstraction_level": "L5 观测层",
        "scope": "ResourceInstance", "lifecycle_type": "事件对象",
        "unique_key": "alert_event_id",
        "key_properties": "alert_event_id, alert_name, severity, status, fired_at",
        "inspection_focus": "告警数量、告警归并、误报噪声、响应时效",
        "health_fields": "status, severity, duration",
        "required_relation_summary": "FIRED_ON ResourceInstance; AGGREGATES_TO Application",
        "import_label": ":ResourceType:AlertEvent",
    },
]

# ============================================================
# 2. 新增类型关系 (REL-042 ~ REL-047)
# ============================================================

L4_TYPE_EDGES = [
    {
        "edge_id": "REL-042", "source_node_id": "RT-020", "source_label": "InspectionRun",
        "relationship_type": "GENERATED", "relationship_name": "生成了",
        "target_node_id": "RT-022", "target_label": "InspectionFinding",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "一次巡检运行产生一组发现",
        "inspection_check_item": "巡检是否完整执行；是否有规则未执行",
        "risk_signal": "巡检运行失败；大量新发现",
        "impact_direction": "Run -> Finding -> Resource",
        "alert_aggregation": "",
        "discovery_method": "巡检引擎回调",
        "graph_view": "巡检视图",
    },
    {
        "edge_id": "REL-043", "source_node_id": "RT-022", "source_label": "InspectionFinding",
        "relationship_type": "VIOLATES", "relationship_name": "违反",
        "target_node_id": "RT-021", "target_label": "InspectionRule",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "发现命中了哪条规则",
        "inspection_check_item": "规则命中率；高命中率规则是否需要调整阈值",
        "risk_signal": "核心规则大量命中",
        "impact_direction": "Finding -> Rule",
        "alert_aggregation": "",
        "discovery_method": "巡检引擎",
        "graph_view": "巡检视图",
    },
    {
        "edge_id": "REL-044", "source_node_id": "RT-022", "source_label": "InspectionFinding",
        "relationship_type": "AFFECTS", "relationship_name": "影响",
        "target_node_id": "RT-015", "target_label": "Pod",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "发现关联到具体资源",
        "inspection_check_item": "Affected 资源是否存在；传播链是否完整",
        "risk_signal": "核心资源关联高危发现",
        "impact_direction": "Finding -> Resource -> upstream to Application",
        "alert_aggregation": "",
        "discovery_method": "巡检引擎 + 图推导",
        "graph_view": "巡检视图",
    },
    {
        "edge_id": "REL-045", "source_node_id": "RT-022", "source_label": "InspectionFinding",
        "relationship_type": "PROPAGATES_TO", "relationship_name": "传播到",
        "target_node_id": "RT-003", "target_label": "ApplicationComponent",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "推导",
        "impact_analysis": "是",
        "inspection_purpose": "影响传播链——Finding 影响 Pod → 传播到 Deployment → Component",
        "inspection_check_item": "影响传播链是否完整",
        "risk_signal": "底层风险向上传播影响业务应用",
        "impact_direction": "Finding -> Pod -> Deployment -> Component -> Application",
        "alert_aggregation": "",
        "discovery_method": "图遍历推导",
        "graph_view": "影响分析视图",
    },
    {
        "edge_id": "REL-046", "source_node_id": "RT-023", "source_label": "AlertEvent",
        "relationship_type": "FIRED_ON", "relationship_name": "触发在",
        "target_node_id": "RT-015", "target_label": "Pod",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "告警触发在某个具体资源上",
        "inspection_check_item": "告警关联资源是否存在；标签匹配是否正确",
        "risk_signal": "同一资源多个告警；同一应用多个资源告警",
        "impact_direction": "Alert -> Resource -> upstream to Application",
        "alert_aggregation": "多 Pod 告警向 Deployment/Component 归并",
        "discovery_method": "AlertManager Webhook + 标签匹配",
        "graph_view": "告警归并视图",
    },
    {
        "edge_id": "REL-047", "source_node_id": "RT-023", "source_label": "AlertEvent",
        "relationship_type": "AGGREGATES_TO", "relationship_name": "归并到",
        "target_node_id": "RT-002", "target_label": "Application",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "推导",
        "impact_analysis": "是",
        "inspection_purpose": "告警按应用归并——判断多个告警是否属于同一应用故障",
        "inspection_check_item": "告警归并是否遗漏；是否过度归并",
        "risk_signal": "单个应用多告警 => 可能是一个根因引发的多条告警",
        "impact_direction": "",
        "alert_aggregation": "Alert -> Pod -> Deployment -> Component -> Application 归并",
        "discovery_method": "图遍历推导",
        "graph_view": "告警归并视图",
    },
]

# ============================================================
# 3. 巡检规则定义
# ============================================================

INSPECTION_RULES = [
    {"rule_id": "rule-001", "rule_name": "Pod CPU 使用率过高",          "category": "resource",     "severity": "warning",  "applies_to": "Pod",            "description": "CPU 使用率超过 80% 阈值", "remediation": "检查应用性能；考虑增加 CPU 限制或水平扩展"},
    {"rule_id": "rule-002", "rule_name": "Pod 频繁重启",               "category": "availability", "severity": "critical", "applies_to": "Pod",            "description": "24h 内重启超过 10 次", "remediation": "检查 CrashLoopBackOff 原因；查看日志；检查资源限制"},
    {"rule_id": "rule-003", "rule_name": "Deployment 副本不一致",       "category": "availability", "severity": "critical", "applies_to": "Deployment",     "description": "期望副本与可用副本不一致", "remediation": "检查 Pod 状态；排查调度问题或资源不足"},
    {"rule_id": "rule-004", "rule_name": "Secret 即将过期",             "category": "security",     "severity": "warning",  "applies_to": "Secret",         "description": "证书/密钥在 14 天内过期", "remediation": "联系密钥管理员轮换；验证轮换后的部署"},
    {"rule_id": "rule-005", "rule_name": "镜像存在高危漏洞",            "category": "security",     "severity": "critical", "applies_to": "ContainerImage", "description": "镜像存在 Critical 级别 CVE", "remediation": "升级镜像到修复版本；或申请漏洞例外"},
    {"rule_id": "rule-006", "rule_name": "Service 无后端 Pod",          "category": "availability", "severity": "critical", "applies_to": "Service",        "description": "Service 选择器无匹配 Pod", "remediation": "检查 selector 标签；确认 Pod 是否存在"},
    {"rule_id": "rule-007", "rule_name": "Ingress TLS 即将过期",       "category": "security",     "severity": "warning",  "applies_to": "Ingress",        "description": "TLS 证书在 14 天内过期", "remediation": "更新 Ingress TLS 证书"},
    {"rule_id": "rule-008", "rule_name": "节点资源压力",                "category": "resource",     "severity": "warning",  "applies_to": "KubernetesNode", "description": "节点 CPU/内存/磁盘压力", "remediation": "检查节点负载；考虑扩容或迁移 Pod"},
    {"rule_id": "rule-009", "rule_name": "ConfigMap 配置漂移",          "category": "config",       "severity": "warning",  "applies_to": "ConfigMap",      "description": "与基线版本不一致", "remediation": "对比配置差异；评估变更风险；决定回滚或更新基线"},
    {"rule_id": "rule-010", "rule_name": "容器以 root 运行",            "category": "security",     "severity": "critical", "applies_to": "Container",      "description": "安全上下文 allowPrivilegeEscalation", "remediation": "设置 runAsNonRoot=true；配置 securityContext"},
]

# ============================================================
# 4. 巡检发现定义
# ============================================================

# Run 1: 部分通过
FINDINGS_RUN1 = [
    {"id": "finding-run1-001", "rule_id": "rule-003", "severity": "warning", "status": "open",
     "resource_id": f"deploy:{CLUSTER_ID}:{NAMESPACE}:order-api", "resource_type": "Deployment", "resource_name": "order-api",
     "description": "Deployment order-api 期望副本 3，可用副本 2",
     "evidence": '{"desired_replicas":3,"available_replicas":2,"current_value":2,"threshold":3}',
     "recommendation": "检查 Pod 状态，排查 CrashLoopBackOff 或资源不足"},
    {"id": "finding-run1-002", "rule_id": "rule-004", "severity": "warning", "status": "open",
     "resource_id": f"secret:{CLUSTER_ID}:{NAMESPACE}:order-api-secret", "resource_type": "Secret", "resource_name": "order-api-secret",
     "description": "Secret order-api-secret 将在 14 天后过期",
     "evidence": '{"expiry_days":14,"expiry_date":"2026-06-29T10:00:00Z","threshold_days":30}',
     "recommendation": "联系密钥管理员轮换；验证轮换后的部署"},
    {"id": "finding-run1-003", "rule_id": "rule-005", "severity": "critical", "status": "open",
     "resource_id": "image:order-api:1.2.3", "resource_type": "ContainerImage", "resource_name": "order-api:1.2.3",
     "description": "镜像 order-api:1.2.3 存在 1 个 Critical 级别 CVE: CVE-2026-1234 (CVSS 9.8)",
     "evidence": '{"critical_vulns":1,"high_vulns":3,"cve_list":[{"id":"CVE-2026-1234","cvss":9.8,"package":"openssl","fixed_version":"1.1.1w"}]}',
     "recommendation": "升级镜像到修复版本 order-api:v1.2.4 或申请漏洞例外"},
]

# Run 2: 更多问题
FINDINGS_RUN2 = [
    {"id": "finding-run2-001", "rule_id": "rule-001", "severity": "warning", "status": "open",
     "resource_id": f"pod:{POD_PREFIX}-6fd9c8b7c9-abcdf", "resource_type": "Pod", "resource_name": "order-api-6fd9c8b7c9-abcdf",
     "description": "Pod order-api-6fd9c8b7c9-abcdf CPU 使用率 86.5% 超过 80% 阈值",
     "evidence": '{"current_value":86.5,"threshold":80,"unit":"percent","measured_at":"2026-06-15T10:03:00Z"}',
     "recommendation": "检查应用性能问题；考虑增加 CPU 限制或水平扩展"},
    {"id": "finding-run2-002", "rule_id": "rule-002", "severity": "warning", "status": "open",
     "resource_id": f"pod:{POD_PREFIX}-6fd9c8b7c9-abcdf", "resource_type": "Pod", "resource_name": "order-api-6fd9c8b7c9-abcdf",
     "description": "Pod order-api-6fd9c8b7c9-abcdf 24h 内重启 3 次",
     "evidence": '{"restart_count_24h":3,"threshold":10,"status":"warning"}',
     "recommendation": "检查应用日志；排查 OOM 或性能瓶颈"},
    {"id": "finding-run2-003", "rule_id": "rule-008", "severity": "warning", "status": "open",
     "resource_id": f"node:{CLUSTER_ID}:worker-02", "resource_type": "KubernetesNode", "resource_name": "worker-02",
     "description": "节点 worker-02 磁盘使用率 72%，接近 80% 阈值",
     "evidence": '{"disk_usage_percent":72,"threshold":80,"filesystem":"/var/lib/docker"}',
     "recommendation": "清理未使用的镜像和容器；考虑扩展节点"},
]

ALL_FINDINGS = FINDINGS_RUN1 + FINDINGS_RUN2

# ============================================================
# 5. 告警事件定义
# ============================================================

ALERT_EVENTS = [
    {
        "alert_event_id": "alert-order-api-error-rate",
        "alert_name": "OrderAPIHighErrorRate",
        "severity": "critical",
        "status": "firing",
        "fired_at": (NOW - timedelta(minutes=15)).isoformat() + "+08:00",
        "resolved_at": "",
        "prometheus_alert_id": "fpr_abc123def456",
        "summary": "订单 API 错误率超过 5%",
        "description": "order-api 组件错误率 7.2% 超过 critical 阈值 5%，持续 15 分钟",
        "affected_labels": '{"namespace":"order","pod":"order-api-6fd9c8b7c9-abcdf","deployment":"order-api","component":"order-api","application":"order"}',
        "silence_url": "https://alertmanager.example.com/#/silences/new?alert=OrderAPIHighErrorRate",
        "dashboard_url": "https://grafana.example.com/d/order-api?var-namespace=order",
        "resource_ref": f"pod:{POD_PREFIX}-6fd9c8b7c9-abcdf",
    },
    {
        "alert_event_id": "alert-pod-restart-loop",
        "alert_name": "PodRestartLoop",
        "severity": "warning",
        "status": "firing",
        "fired_at": (NOW - timedelta(minutes=30)).isoformat() + "+08:00",
        "resolved_at": "",
        "prometheus_alert_id": "fpr_def789ghi012",
        "summary": "Pod order-api-6fd9c8b7c9-abcdl 频繁重启",
        "description": "Pod order-api-6fd9c8b7c9-abcdl 在过去 1 小时内重启了 2 次",
        "affected_labels": '{"namespace":"order","pod":"order-api-6fd9c8b7c9-abcdl","deployment":"order-api","component":"order-api","application":"order"}',
        "silence_url": "",
        "dashboard_url": "",
        "resource_ref": f"pod:{POD_PREFIX}-6fd9c8b7c9-abcdl",
    },
    {
        "alert_event_id": "alert-node-disk-pressure",
        "alert_name": "NodeDiskPressure",
        "severity": "warning",
        "status": "firing",
        "fired_at": (NOW - timedelta(hours=1)).isoformat() + "+08:00",
        "resolved_at": "",
        "prometheus_alert_id": "fpr_jkl345mno678",
        "summary": "节点 worker-02 磁盘空间不足",
        "description": "节点 worker-02 磁盘使用率 72%，文件系统 /var/lib/docker 空间不足",
        "affected_labels": '{"node":"worker-02","cluster":"cce-prod-01"}',
        "silence_url": "",
        "dashboard_url": "https://grafana.example.com/d/nodes?var-node=worker-02",
        "resource_ref": f"node:{CLUSTER_ID}:worker-02",
    },
    {
        "alert_event_id": "alert-tls-cert-expiring",
        "alert_name": "TLSCertExpiring",
        "severity": "warning",
        "status": "firing",
        "fired_at": (NOW - timedelta(hours=2)).isoformat() + "+08:00",
        "resolved_at": "",
        "prometheus_alert_id": "fpr_pqr901stu234",
        "summary": "Ingress TLS 证书即将过期",
        "description": "order.example.com 的 TLS 证书将在 14 天后过期",
        "affected_labels": '{"host":"order.example.com","namespace":"order","ingress":"order-api-ing"}',
        "silence_url": "",
        "dashboard_url": "",
        "resource_ref": f"ing:{CLUSTER_ID}:{NAMESPACE}:order-api",
    },
    {
        "alert_event_id": "alert-deploy-replicas-mismatch",
        "alert_name": "DeploymentReplicasMismatch",
        "severity": "warning",
        "status": "resolved",
        "fired_at": (NOW - timedelta(hours=6)).isoformat() + "+08:00",
        "resolved_at": (NOW - timedelta(hours=3)).isoformat() + "+08:00",
        "prometheus_alert_id": "fpr_vwx567yza890",
        "summary": "Deployment order-api 副本不一致",
        "description": "Deployment order-api 期望 3 副本，实际 2 副本可用（已恢复）",
        "affected_labels": '{"namespace":"order","deployment":"order-api","component":"order-api","application":"order"}',
        "silence_url": "",
        "dashboard_url": "",
        "resource_ref": f"deploy:{CLUSTER_ID}:{NAMESPACE}:order-api",
    },
]

# ============================================================
# 6. 生成实例节点和关系
# ============================================================

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


def generate_l4_instance_nodes():
    rows = []

    # --- InspectionRuns ---
    run1_start = (NOW - timedelta(minutes=35)).isoformat() + "+08:00"
    run1_end = (NOW - timedelta(minutes=30)).isoformat() + "+08:00"

    rows.append({
        "node_id": "run-20260615-001", "label": "InspectionRun",
        "name": "生产环境定时巡检 #20260615-001", "unique_key": "run-20260615-001",
        "env_code": "prod", "app_code": "order", "component_code": "",
        "cluster_id": CLUSTER_ID, "namespace": "",
        "owner_team": "SRE", "lifecycle_status": "completed",
        "health_status": "normal", "risk_level": "low",
        "inspection_status": "passed",
        "last_inspected_at": run1_end,
        "source_system": "InspectionEngine", "source_ref": "runs/20260615-001",
        "attrs_json": f'{{"run_type":"scheduled","scope":"prod/order","started_at":"{run1_start}","completed_at":"{run1_end}","duration_seconds":300,"total_rules":10,"passed_rules":7,"failed_rules":2,"skipped_rules":1,"overall_status":"passed","triggered_by":"cron:0 */6 * * *"}}',
    })

    run2_start = (NOW - timedelta(minutes=5)).isoformat() + "+08:00"
    run2_end = NOW_ISO
    rows.append({
        "node_id": "run-20260615-002", "label": "InspectionRun",
        "name": "生产环境定时巡检 #20260615-002", "unique_key": "run-20260615-002",
        "env_code": "prod", "app_code": "order", "component_code": "",
        "cluster_id": CLUSTER_ID, "namespace": "",
        "owner_team": "SRE", "lifecycle_status": "completed",
        "health_status": "warning", "risk_level": "medium",
        "inspection_status": "failed",
        "last_inspected_at": run2_end,
        "source_system": "InspectionEngine", "source_ref": "runs/20260615-002",
        "attrs_json": f'{{"run_type":"scheduled","scope":"prod/order","started_at":"{run2_start}","completed_at":"{run2_end}","duration_seconds":300,"total_rules":10,"passed_rules":6,"failed_rules":3,"skipped_rules":1,"overall_status":"warning","triggered_by":"cron:0 */6 * * *"}}',
    })

    # --- InspectionRules ---
    for r in INSPECTION_RULES:
        rows.append({
            "node_id": r["rule_id"], "label": "InspectionRule",
            "name": r["rule_name"], "unique_key": r["rule_id"],
            "env_code": "", "app_code": "", "component_code": "",
            "cluster_id": "", "namespace": "",
            "owner_team": "SRE", "lifecycle_status": "active",
            "health_status": "normal", "risk_level": "low",
            "inspection_status": "",
            "last_inspected_at": "",
            "source_system": "InspectionEngine", "source_ref": f"rules/{r['rule_id']}",
            "attrs_json": f'{{"rule_category":"{r["category"]}","severity":"{r["severity"]}","applies_to_resource_type":"{r["applies_to"]}","description":"{r["description"]}","remediation":"{r["remediation"]}","enabled":true}}',
        })

    # --- InspectionFindings ---
    for f in ALL_FINDINGS:
        rows.append({
            "node_id": f["id"], "label": "InspectionFinding",
            "name": f["description"][:80], "unique_key": f["id"],
            "env_code": "prod", "app_code": "order", "component_code": "order-api",
            "cluster_id": CLUSTER_ID, "namespace": NAMESPACE,
            "owner_team": "SRE", "lifecycle_status": "active",
            "health_status": "warning" if f["severity"] == "warning" else "critical",
            "risk_level": "medium" if f["severity"] == "warning" else "high",
            "inspection_status": "failed",
            "last_inspected_at": NOW_ISO,
            "source_system": "InspectionEngine", "source_ref": f"findings/{f['id']}",
            "attrs_json": f'{{"rule_id":"{f["rule_id"]}","rule_name":"{next(r["rule_name"] for r in INSPECTION_RULES if r["rule_id"] == f["rule_id"])}","severity":"{f["severity"]}","status":"{f["status"]}","affected_resource_id":"{f["resource_id"]}","affected_resource_type":"{f["resource_type"]}","affected_resource_name":"{f["resource_name"]}","description":"{f["description"]}","evidence":{f["evidence"]},"recommendation":"{f["recommendation"]}","detected_at":"{NOW_ISO}"}}',
        })

    # --- AlertEvents ---
    for a in ALERT_EVENTS:
        app_ref = ""
        if "order" in (a.get("affected_labels") or ""):
            app_ref = "order"
        rows.append({
            "node_id": a["alert_event_id"], "label": "AlertEvent",
            "name": a["alert_name"], "unique_key": a["alert_event_id"],
            "env_code": "prod", "app_code": app_ref, "component_code": "order-api",
            "cluster_id": CLUSTER_ID, "namespace": NAMESPACE,
            "owner_team": "SRE", "lifecycle_status": "active",
            "health_status": "normal" if a["status"] == "resolved" else "critical",
            "risk_level": "medium" if a["severity"] == "warning" else "high",
            "inspection_status": "",
            "last_inspected_at": "",
            "source_system": "AlertManager", "source_ref": f"alerts/{a['prometheus_alert_id']}",
            "attrs_json": f'{{"alert_name":"{a["alert_name"]}","severity":"{a["severity"]}","status":"{a["status"]}","fired_at":"{a["fired_at"]}","resolved_at":"{a["resolved_at"]}","summary":"{a["summary"]}","description":"{a["description"]}","affected_labels":{a["affected_labels"]},"resource_ref":"{a["resource_ref"]}"}}',
        })

    return rows


def generate_l4_instance_edges():
    rows = []
    eid = 200

    # Run 1 → Findings
    for f in FINDINGS_RUN1:
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": "run-20260615-001",
            "relationship_type": "GENERATED",
            "target_node_id": f["id"],
            "relationship_name": "生成了",
            "dependency_strength": "强", "is_required": "是",
            "discovery_method": "巡检引擎",
            "health_status": "normal", "risk_signal": "",
            "last_verified_at": NOW_ISO, "attrs_json": "{}",
        })

    # Run 2 → Findings
    for f in FINDINGS_RUN2:
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": "run-20260615-002",
            "relationship_type": "GENERATED",
            "target_node_id": f["id"],
            "relationship_name": "生成了",
            "dependency_strength": "强", "is_required": "是",
            "discovery_method": "巡检引擎",
            "health_status": "normal", "risk_signal": "",
            "last_verified_at": NOW_ISO, "attrs_json": "{}",
        })

    # Finding → VIOLATES Rule
    for f in ALL_FINDINGS:
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": f["id"],
            "relationship_type": "VIOLATES",
            "target_node_id": f["rule_id"],
            "relationship_name": "违反",
            "dependency_strength": "强", "is_required": "是",
            "discovery_method": "巡检引擎",
            "health_status": "normal", "risk_signal": "",
            "last_verified_at": NOW_ISO, "attrs_json": "{}",
        })

    # Finding → AFFECTS Resource
    for f in ALL_FINDINGS:
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": f["id"],
            "relationship_type": "AFFECTS",
            "target_node_id": f["resource_id"],
            "relationship_name": "影响",
            "dependency_strength": "强", "is_required": "是",
            "discovery_method": "巡检引擎",
            "health_status": "warning", "risk_signal": f["description"][:100],
            "last_verified_at": NOW_ISO, "attrs_json": "{}",
        })

    # Finding → PROPAGATES_TO (upstream to ApplicationComponent or Application)
    propagated = set()
    for f in ALL_FINDINGS:
        if f["resource_type"] in ("Pod", "Deployment"):
            target = "comp:order-api"
        elif f["resource_type"] in ("Secret", "ContainerImage"):
            target = "app:order"
        else:
            continue
        key = (f["id"], target)
        if key in propagated:
            continue
        propagated.add(key)
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": f["id"],
            "relationship_type": "PROPAGATES_TO",
            "target_node_id": target,
            "relationship_name": "传播到",
            "dependency_strength": "中", "is_required": "否",
            "discovery_method": "图推导",
            "health_status": "warning", "risk_signal": "风险向上传播",
            "last_verified_at": NOW_ISO, "attrs_json": '{"derived":true}',
        })

    # AlertEvent → FIRED_ON Resource
    for a in ALERT_EVENTS:
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": a["alert_event_id"],
            "relationship_type": "FIRED_ON",
            "target_node_id": a["resource_ref"],
            "relationship_name": "触发在",
            "dependency_strength": "强", "is_required": "是",
            "discovery_method": "AlertManager",
            "health_status": "warning", "risk_signal": a["summary"],
            "last_verified_at": NOW_ISO, "attrs_json": "{}",
        })

    # AlertEvent → AGGREGATES_TO Application
    for a in ALERT_EVENTS:
        eid += 1
        rows.append({
            "edge_id": f"e{eid:03d}",
            "source_node_id": a["alert_event_id"],
            "relationship_type": "AGGREGATES_TO",
            "target_node_id": "app:order",
            "relationship_name": "归并到",
            "dependency_strength": "中", "is_required": "否",
            "discovery_method": "图推导",
            "health_status": "normal", "risk_signal": "",
            "last_verified_at": NOW_ISO, "attrs_json": '{"derived":true}',
        })

    return rows


# ============================================================
# 7. 写入
# ============================================================

L4_TYPE_COLS = [
    "node_id", "node_name", "node_label", "node_group", "abstraction_level",
    "scope", "lifecycle_type", "unique_key", "key_properties",
    "inspection_focus", "health_fields", "required_relation_summary", "import_label",
]

L4_TYPE_EDGE_COLS = [
    "edge_id", "source_node_id", "source_node_name", "source_label",
    "relationship_type", "relationship_name",
    "target_node_id", "target_node_name", "target_label",
    "dependency_strength", "is_required", "auto_discovery",
    "impact_analysis", "inspection_purpose", "inspection_check_item",
    "risk_signal", "impact_direction", "alert_aggregation",
    "discovery_method", "graph_view",
]


def write_csv(filename: str, columns: list, rows: list):
    filepath = os.path.join(OUTPUT_DIR, filename)
    with open(filepath, "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=columns, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    print(f"  ✓ {filename} ({len(rows)} rows)")


def main():
    print("Generating L4 Mock Data...")

    # Type extensions aren't stored as ResourceType nodes in the instance DB;
    # they are reference documents. But output CSV for completeness.
    write_csv("l4_type_extensions.csv", L4_TYPE_COLS, L4_TYPE_NODES)
    write_csv("l4_type_relationship_extensions.csv", L4_TYPE_EDGE_COLS, L4_TYPE_EDGES)

    nodes = generate_l4_instance_nodes()
    write_csv("l4_instance_nodes.csv", INSTANCE_NODE_COLUMNS, nodes)

    edges = generate_l4_instance_edges()
    write_csv("l4_instance_edges.csv", INSTANCE_EDGE_COLUMNS, edges)

    total = len(nodes) + len(edges)
    print(f"\nTotal L4 records: {total}")
    print("Done.")


if __name__ == "__main__":
    main()
