#!/usr/bin/env python3
"""
Recovery Action Mock Data Generator — PRD-001 Sprint 1

输出到 scripts/output/:
  - recovery_type_extensions.csv             新增类型节点 (RT-024 ~ RT-026)
  - recovery_type_relationship_extensions.csv 新增类型关系 (REL-048 ~ REL-054)
  - recovery_instance_nodes.csv              8 种 RecoveryAction 模板实例
  - recovery_instance_edges.csv              SUGGESTS / EXECUTES_ON 关系

PRD-001 Sprint 1 范围:
  - 8 种动作模板入图
  - InspectionFinding -SUGGESTS-> RecoveryAction(基于现有 finding 的 rule_id 映射)
  - RecoveryAction -EXECUTES_ON-> ResourceType(类型层关系,作为查询索引)
  - 暂不生成 RecoveryExecution / ApprovalRequest 实例(等真实操作时由 API 创建)
"""

import csv
import json
import os

OUTPUT_DIR = os.path.join(os.path.dirname(__file__), "output")
os.makedirs(OUTPUT_DIR, exist_ok=True)

NOW_ISO = "2026-06-15T10:00:00+08:00"


# ============================================================
# 1. 新增类型节点 (RT-024 ~ RT-026)
# ============================================================

RECOVERY_TYPE_NODES = [
    {
        "node_id": "RT-024", "node_name": "恢复动作", "node_label": "RecoveryAction",
        "node_group": "快恢", "abstraction_level": "L7 快恢层",
        "scope": "Global/ResourceType", "lifecycle_type": "配置对象",
        "unique_key": "action_id",
        "key_properties": "action_id, action_name, action_category, target_resource_type, risk_level, requires_approval, rollback_action_id",
        "inspection_focus": "动作覆盖率、风险分级合规、审批触发率",
        "health_fields": "enabled_status, last_executed_at",
        "required_relation_summary": "EXECUTES_ON ResourceType; ROLLBACK_OF RecoveryAction; SUGGESTED_BY InspectionFinding",
        "import_label": ":ResourceType:RecoveryAction",
    },
    {
        "node_id": "RT-025", "node_name": "恢复动作执行", "node_label": "RecoveryExecution",
        "node_group": "快恢", "abstraction_level": "L7 快恢层",
        "scope": "ResourceInstance", "lifecycle_type": "事件对象",
        "unique_key": "execution_id",
        "key_properties": "execution_id, action_id, target_resource_id, status, initiated_by, initiated_at",
        "inspection_focus": "执行成功率、平均耗时、回滚率、审批合规",
        "health_fields": "status, duration_seconds",
        "required_relation_summary": "TARGETS ResourceInstance; TRIGGERED_BY InspectionFinding; REQUIRES_APPROVAL ApprovalRequest; ROLLED_BACK_BY RecoveryExecution",
        "import_label": ":ResourceType:RecoveryExecution",
    },
    {
        "node_id": "RT-026", "node_name": "审批请求", "node_label": "ApprovalRequest",
        "node_group": "快恢", "abstraction_level": "L7 快恢层",
        "scope": "RecoveryExecution", "lifecycle_type": "事件对象",
        "unique_key": "approval_id",
        "key_properties": "approval_id, execution_id, approver_id, approval_status, requested_at",
        "inspection_focus": "审批时效、批准率、过期率、超时率",
        "health_fields": "approval_status, time_to_approve",
        "required_relation_summary": "APPROVES RecoveryExecution",
        "import_label": ":ResourceType:ApprovalRequest",
    },
]


# ============================================================
# 2. 新增类型关系 (REL-048 ~ REL-054)
# ============================================================

RECOVERY_TYPE_EDGES = [
    {
        "edge_id": "REL-048",
        "source_node_id": "RT-022", "source_label": "InspectionFinding",
        "relationship_type": "SUGGESTS", "relationship_name": "推荐",
        "target_node_id": "RT-024", "target_label": "RecoveryAction",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "推导",
        "impact_analysis": "是",
        "inspection_purpose": "Finding 关联到推荐恢复动作",
        "inspection_check_item": "Finding 是否有可推荐动作; 推荐准确率; 是否被采纳",
        "risk_signal": "无可推荐动作的 critical Finding",
        "impact_direction": "Finding -> RecoveryAction",
        "alert_aggregation": "",
        "discovery_method": "规则匹配", "graph_view": "快恢决策视图",
    },
    {
        "edge_id": "REL-049",
        "source_node_id": "RT-024", "source_label": "RecoveryAction",
        "relationship_type": "EXECUTES_ON", "relationship_name": "可执行于",
        "target_node_id": "RT-XXX", "target_label": "ResourceType",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "类型层动作适用性",
        "inspection_check_item": "动作目标类型与实际资源是否匹配",
        "risk_signal": "类型不匹配错误执行",
        "impact_direction": "Action -> ResourceType",
        "alert_aggregation": "",
        "discovery_method": "动作模板配置", "graph_view": "快恢决策视图",
    },
    {
        "edge_id": "REL-050",
        "source_node_id": "RT-025", "source_label": "RecoveryExecution",
        "relationship_type": "TARGETS", "relationship_name": "针对",
        "target_node_id": "RT-XXX", "target_label": "ResourceInstance",
        "dependency_strength": "强", "is_required": "是", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "执行实例关联到具体资源",
        "inspection_check_item": "目标资源是否仍存在;执行后状态变化",
        "risk_signal": "执行失败的核心资源",
        "impact_direction": "Execution -> Resource",
        "alert_aggregation": "",
        "discovery_method": "API 调用", "graph_view": "快恢决策视图",
    },
    {
        "edge_id": "REL-051",
        "source_node_id": "RT-025", "source_label": "RecoveryExecution",
        "relationship_type": "TRIGGERED_BY", "relationship_name": "触发自",
        "target_node_id": "RT-022", "target_label": "InspectionFinding",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "执行可追溯到具体 Finding",
        "inspection_check_item": "Finding-动作转化率;Finding 解决率",
        "risk_signal": "Finding 被关闭但执行失败",
        "impact_direction": "Execution -> Finding",
        "alert_aggregation": "",
        "discovery_method": "API 调用上下文", "graph_view": "快恢决策视图",
    },
    {
        "edge_id": "REL-052",
        "source_node_id": "RT-025", "source_label": "RecoveryExecution",
        "relationship_type": "REQUIRES_APPROVAL", "relationship_name": "需审批",
        "target_node_id": "RT-026", "target_label": "ApprovalRequest",
        "dependency_strength": "强", "is_required": "条件性", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "high_risk 动作必须有审批记录",
        "inspection_check_item": "high_risk 动作无审批 = 违规",
        "risk_signal": "high_risk 未审批直接执行",
        "impact_direction": "Execution -> ApprovalRequest",
        "alert_aggregation": "",
        "discovery_method": "动作风险分级", "graph_view": "快恢决策视图",
    },
    {
        "edge_id": "REL-053",
        "source_node_id": "RT-026", "source_label": "ApprovalRequest",
        "relationship_type": "APPROVED_BY", "relationship_name": "审批人",
        "target_node_id": "RT-XXX", "target_label": "User",
        "dependency_strength": "强", "is_required": "条件性", "auto_discovery": "自动",
        "impact_analysis": "否",
        "inspection_purpose": "审批责任链",
        "inspection_check_item": "审批人权限合规",
        "risk_signal": "无权限用户审批高风险动作",
        "impact_direction": "ApprovalRequest -> User",
        "alert_aggregation": "",
        "discovery_method": "RBAC", "graph_view": "快恢决策视图",
    },
    {
        "edge_id": "REL-054",
        "source_node_id": "RT-025", "source_label": "RecoveryExecution",
        "relationship_type": "ROLLED_BACK_BY", "relationship_name": "被回滚",
        "target_node_id": "RT-025", "target_label": "RecoveryExecution",
        "dependency_strength": "中", "is_required": "否", "auto_discovery": "自动",
        "impact_analysis": "是",
        "inspection_purpose": "回滚双向链接",
        "inspection_check_item": "回滚执行成功率",
        "risk_signal": "回滚也失败的连环故障",
        "impact_direction": "Execution -> Execution(双向)",
        "alert_aggregation": "",
        "discovery_method": "用户主动回滚", "graph_view": "快恢决策视图",
    },
]


# ============================================================
# 3. 8 种动作模板定义 (PRD-001 V1)
# ============================================================

RECOVERY_ACTIONS = [
    {
        "action_id": "restart_pod",
        "action_name": "重启 Pod",
        "action_category": "availability",
        "target_resource_type": "Pod",
        "risk_level": "medium",
        "requires_approval": True,
        "rollback_action_id": None,    # 重启没有简单回滚(只能等下一次启动失败)
        "input_schema": {
            "type": "object",
            "properties": {
                "graceful": {"type": "boolean", "default": True, "description": "是否优雅终止(SIGTERM 后等待)"},
                "grace_period_seconds": {"type": "integer", "default": 30, "minimum": 0, "maximum": 300},
            },
        },
        "description": "对目标 Pod 执行 kubectl delete pod,触发其 ReplicaSet 自动重新调度。期间该 Pod 提供的服务短暂不可用。",
        "estimated_duration_seconds": 60,
        "dry_run_handler": "app.recovery.handlers.restart_pod.dry_run",
        "execute_handler": "app.recovery.handlers.restart_pod.execute",
    },
    {
        "action_id": "scale_deployment",
        "action_name": "调整 Deployment 副本",
        "action_category": "scale",
        "target_resource_type": "Deployment",
        "risk_level": "low",
        "requires_approval": False,
        "rollback_action_id": "scale_deployment",    # 反向 delta 即可
        "input_schema": {
            "type": "object",
            "properties": {
                "replicas_delta": {"type": "integer", "default": 1, "minimum": -10, "maximum": 10,
                                   "description": "副本数变化量,正数扩容,负数缩容"},
            },
            "required": ["replicas_delta"],
        },
        "description": "对 Deployment 增减副本数。扩容用于缓解资源压力,缩容用于节省成本。新副本启动时间由镜像决定。",
        "estimated_duration_seconds": 90,
        "dry_run_handler": "app.recovery.handlers.scale_deployment.dry_run",
        "execute_handler": "app.recovery.handlers.scale_deployment.execute",
    },
    {
        "action_id": "rollback_deployment",
        "action_name": "回滚 Deployment 版本",
        "action_category": "rollback",
        "target_resource_type": "Deployment",
        "risk_level": "high",
        "requires_approval": True,
        "rollback_action_id": "rollback_deployment",    # 再 rollout undo 一次回到之前
        "input_schema": {
            "type": "object",
            "properties": {
                "revision": {"type": "integer", "minimum": 1,
                             "description": "回滚到指定 revision,缺省则回退到上一个"},
            },
        },
        "description": "执行 kubectl rollout undo,把 Deployment 回退到上一个版本(或指定 revision)。会触发滚动重启,期间部分实例不可用。",
        "estimated_duration_seconds": 180,
        "dry_run_handler": "app.recovery.handlers.rollback_deployment.dry_run",
        "execute_handler": "app.recovery.handlers.rollback_deployment.execute",
    },
    {
        "action_id": "refresh_secret",
        "action_name": "刷新 Secret",
        "action_category": "config",
        "target_resource_type": "Secret",
        "risk_level": "medium",
        "requires_approval": True,
        "rollback_action_id": None,    # 旧 Secret 一旦覆盖无法回滚(应记录历史值)
        "input_schema": {
            "type": "object",
            "properties": {
                "trigger_pod_restart": {"type": "boolean", "default": True,
                                        "description": "刷新后是否触发使用此 Secret 的 Pod 滚动重启"},
            },
        },
        "description": "更新 Secret 内容并(可选)滚动重启所有引用此 Secret 的 Pod。常用于密钥过期前轮换。",
        "estimated_duration_seconds": 300,
        "dry_run_handler": "app.recovery.handlers.refresh_secret.dry_run",
        "execute_handler": "app.recovery.handlers.refresh_secret.execute",
    },
    {
        "action_id": "drain_node",
        "action_name": "驱逐 Node 上的 Pod",
        "action_category": "drain",
        "target_resource_type": "KubernetesNode",
        "risk_level": "high",
        "requires_approval": True,
        "rollback_action_id": None,    # 驱逐后只能 uncordon,不能撤销 Pod 迁移
        "input_schema": {
            "type": "object",
            "properties": {
                "ignore_daemonsets": {"type": "boolean", "default": True},
                "delete_local_data": {"type": "boolean", "default": False},
                "force": {"type": "boolean", "default": False, "description": "强制删除即使无 controller"},
            },
        },
        "description": "对 KubernetesNode 执行 cordon + drain,将其上 Pod 迁移到其他节点。常用于节点维护、磁盘压力或内存泄漏预防。期间该节点上 Pod 短暂不可用。",
        "estimated_duration_seconds": 600,
        "dry_run_handler": "app.recovery.handlers.drain_node.dry_run",
        "execute_handler": "app.recovery.handlers.drain_node.execute",
    },
    {
        "action_id": "kill_query",
        "action_name": "终止 MySQL 慢查询",
        "action_category": "other",
        "target_resource_type": "MySQL",
        "risk_level": "medium",
        "requires_approval": False,
        "rollback_action_id": None,    # 查询终止不可逆
        "input_schema": {
            "type": "object",
            "properties": {
                "query_id": {"type": "string", "description": "MySQL processlist 中的连接 ID"},
                "min_duration_seconds": {"type": "integer", "default": 30,
                                         "description": "只杀掉持续超过此秒数的查询"},
            },
            "required": ["query_id"],
        },
        "description": "对指定 MySQL 实例执行 KILL QUERY,终止特定连接的当前 SQL。用于缓解慢查询引发的锁等待或连接堆积。",
        "estimated_duration_seconds": 5,
        "dry_run_handler": "app.recovery.handlers.kill_query.dry_run",
        "execute_handler": "app.recovery.handlers.kill_query.execute",
    },
    {
        "action_id": "restart_service",
        "action_name": "重启 Service Endpoints",
        "action_category": "availability",
        "target_resource_type": "Service",
        "risk_level": "low",
        "requires_approval": False,
        "rollback_action_id": None,    # Endpoints 自动恢复,无需回滚
        "input_schema": {
            "type": "object",
            "properties": {
                "drop_idle_seconds": {"type": "integer", "default": 0,
                                      "description": "Service 处于 idle 多久后才允许重启"},
            },
        },
        "description": "重新生成 Service 的 Endpoints,触发 kube-proxy 同步 iptables。用于 Service 选择器异常或 Endpoints 缓存陈旧。",
        "estimated_duration_seconds": 30,
        "dry_run_handler": "app.recovery.handlers.restart_service.dry_run",
        "execute_handler": "app.recovery.handlers.restart_service.execute",
    },
    {
        "action_id": "clear_cache",
        "action_name": "清空 Redis 缓存",
        "action_category": "other",
        "target_resource_type": "Redis",
        "risk_level": "medium",
        "requires_approval": True,
        "rollback_action_id": None,    # 缓存清空不可逆
        "input_schema": {
            "type": "object",
            "properties": {
                "scope": {"type": "string", "enum": ["all", "db", "pattern"], "default": "pattern"},
                "db_index": {"type": "integer", "default": 0, "minimum": 0, "maximum": 15},
                "key_pattern": {"type": "string", "description": "scope=pattern 时的 SCAN MATCH 模式"},
            },
        },
        "description": "对 Redis 执行 FLUSHDB / SCAN+DEL,清除指定范围的缓存。会引发短暂缓存击穿。生产环境只允许 pattern 模式。",
        "estimated_duration_seconds": 60,
        "dry_run_handler": "app.recovery.handlers.clear_cache.dry_run",
        "execute_handler": "app.recovery.handlers.clear_cache.execute",
    },
]


# ============================================================
# 4. Finding -> Action 推荐映射(基于 generate_l4 中的 rule_id)
# ============================================================
# 现有 InspectionRule(rule-001 ~ rule-010)与 RecoveryAction 的推荐关系:
#   rule-001 (Pod CPU 高)        -> scale_deployment(增容) + restart_pod(临时缓解)
#   rule-002 (Pod 频繁重启)      -> rollback_deployment(回滚镜像) + restart_pod
#   rule-003 (Deployment 副本)   -> scale_deployment(补足) + rollback_deployment
#   rule-004 (Secret 过期)       -> refresh_secret
#   rule-005 (镜像高危)          -> rollback_deployment(回滚到旧版)
#   rule-006 (Service 无后端)    -> restart_service
#   rule-007 (Ingress TLS)       -> refresh_secret
#   rule-008 (节点压力)          -> drain_node
#   rule-009 (ConfigMap 漂移)    -> rollback_deployment
#   rule-010 (容器以 root)       -> rollback_deployment

RULE_ACTION_MAP = {
    "rule-001": [("scale_deployment", "Pod CPU 高且单副本 → 水平扩容缓解", 0.85),
                 ("restart_pod", "重启可能临时缓解,不解决根因", 0.45)],
    "rule-002": [("rollback_deployment", "频繁重启常因新版本 bug → 回滚版本", 0.75),
                 ("restart_pod", "短期止血,需配合根因排查", 0.40)],
    "rule-003": [("scale_deployment", "副本不足 → 直接补到期望值", 0.90),
                 ("rollback_deployment", "若是新版本启动失败 → 回滚", 0.55)],
    "rule-004": [("refresh_secret", "Secret 过期 → 直接轮换", 0.95)],
    "rule-005": [("rollback_deployment", "高危 CVE → 回滚到无漏洞版本", 0.80)],
    "rule-006": [("restart_service", "Service 无 Endpoints → 重新同步", 0.70)],
    "rule-007": [("refresh_secret", "TLS 即将过期 → 轮换证书", 0.95)],
    "rule-008": [("drain_node", "节点压力 → 驱逐 Pod 到其他节点", 0.70)],
    "rule-009": [("rollback_deployment", "ConfigMap 漂移 → 回滚配置 + 滚动重启", 0.65)],
    "rule-010": [("rollback_deployment", "镜像安全配置错误 → 回滚到合规版本", 0.50)],
}


# ============================================================
# 5. 列定义(对齐 generate_l3/l4 风格)
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

INSTANCE_NODE_COLUMNS = [
    "node_id", "label", "name", "unique_key",
    "env_code", "app_code", "component_code", "cluster_id", "namespace",
    "owner_team", "lifecycle_status", "health_status", "risk_level",
    "inspection_status", "last_inspected_at", "source_system", "source_ref", "attrs_json",
]

INSTANCE_EDGE_COLUMNS = [
    "edge_id", "source_node_id", "relationship_type", "target_node_id",
    "relationship_name", "dependency_strength", "is_required", "discovery_method",
    "health_status", "risk_signal", "last_verified_at", "attrs_json",
]


# ============================================================
# 6. 生成实例
# ============================================================

def generate_recovery_instance_nodes() -> list[dict]:
    """8 种 RecoveryAction 模板作为实例节点入图"""
    rows = []
    for a in RECOVERY_ACTIONS:
        attrs = {
            "action_category": a["action_category"],
            "target_resource_type": a["target_resource_type"],
            "risk_level": a["risk_level"],
            "requires_approval": a["requires_approval"],
            "rollback_action_id": a["rollback_action_id"],
            "input_schema": a["input_schema"],
            "description": a["description"],
            "estimated_duration_seconds": a["estimated_duration_seconds"],
            "dry_run_handler": a["dry_run_handler"],
            "execute_handler": a["execute_handler"],
            "enabled": True,
            "version": "v1",
        }
        rows.append({
            "node_id": a["action_id"],
            "label": "RecoveryAction",
            "name": a["action_name"],
            "unique_key": a["action_id"],
            "env_code": "", "app_code": "", "component_code": "",
            "cluster_id": "", "namespace": "",
            "owner_team": "SRE",
            "lifecycle_status": "active",
            "health_status": "normal",
            "risk_level": a["risk_level"],
            "inspection_status": "",
            "last_inspected_at": "",
            "source_system": "RecoveryEngine",
            "source_ref": f"actions/{a['action_id']}",
            "attrs_json": json.dumps(attrs, ensure_ascii=False),
        })
    return rows


def generate_recovery_instance_edges() -> list[dict]:
    """生成关系:
      1. InspectionFinding -SUGGESTS-> RecoveryAction(基于 RULE_ACTION_MAP)
      2. RecoveryAction -EXECUTES_ON-> ResourceType
         (注:ResourceType 实例节点目前没单独生成,这里用 label 标记关系语义)
    """
    edges = []
    eid = 0

    # 1. Finding → SUGGESTS → Action(读 generate_l4 中所有 finding 的 rule_id)
    # 这里复用 generate_l4 的 finding 列表;为避免循环依赖,直接硬编码 finding-rule 对照
    finding_rule_pairs = [
        ("finding-run1-001", "rule-003"),
        ("finding-run1-002", "rule-004"),
        ("finding-run1-003", "rule-005"),
        ("finding-run2-001", "rule-001"),
        ("finding-run2-002", "rule-002"),
        ("finding-run2-003", "rule-008"),
    ]

    for finding_id, rule_id in finding_rule_pairs:
        suggestions = RULE_ACTION_MAP.get(rule_id, [])
        for action_id, rationale, confidence in suggestions:
            eid += 1
            edges.append({
                "edge_id": f"r{eid:03d}",
                "source_node_id": finding_id,
                "relationship_type": "SUGGESTS",
                "target_node_id": action_id,
                "relationship_name": "推荐",
                "dependency_strength": "中", "is_required": "否",
                "discovery_method": "规则匹配",
                "health_status": "normal",
                "risk_signal": "",
                "last_verified_at": NOW_ISO,
                "attrs_json": json.dumps({
                    "rule_id": rule_id,
                    "rationale": rationale,
                    "confidence": confidence,
                }, ensure_ascii=False),
            })

    return edges


# ============================================================
# 7. 写入
# ============================================================

def write_csv(filename: str, columns: list, rows: list):
    filepath = os.path.join(OUTPUT_DIR, filename)
    with open(filepath, "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=columns, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    print(f"  ✓ {filename} ({len(rows)} rows)")


def main():
    print("Generating Recovery Action Mock Data (PRD-001 Sprint 1)...")

    write_csv("recovery_type_extensions.csv", L4_TYPE_COLS, RECOVERY_TYPE_NODES)
    write_csv("recovery_type_relationship_extensions.csv", L4_TYPE_EDGE_COLS, RECOVERY_TYPE_EDGES)

    nodes = generate_recovery_instance_nodes()
    write_csv("recovery_instance_nodes.csv", INSTANCE_NODE_COLUMNS, nodes)

    edges = generate_recovery_instance_edges()
    write_csv("recovery_instance_edges.csv", INSTANCE_EDGE_COLUMNS, edges)

    total = len(nodes) + len(edges)
    print(f"\nTotal Recovery records: {total} ({len(nodes)} action templates + {len(edges)} suggest edges)")
    print("Done.")


if __name__ == "__main__":
    main()
