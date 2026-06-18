"""8 种 RecoveryAction 模板定义 + 影响传播规则。

这是 backend 运行时的 single source of truth。`scripts/generate_recovery_actions.py`
会另外维护一份用于 mock 数据生成,两者保持核心字段(action_id / risk_level /
requires_approval)一致即可。

**结构**:
- 元数据(对齐 `RecoveryAction` dataclass):name / category / target_type / risk_level / ...
- 影响传播(给 `cascade.py` dry-run 用):propagation 列表,每条描述一种关系遍历
  * edge:关系类型(CONTAINS / USES / ROUTES_TO 等)
  * direction:"forward"(顺着关系)| "reverse"(逆着关系)
  * max_depth:遍历深度(避免深递归)
  * target_type:筛选目标节点类型(可选)
  * impact:"minimal" | "low" | "medium" | "high" — 受影响节点的严重度等级
  * note:人类可读的影响描述
"""

ACTION_DEFS: dict[str, dict] = {
    "restart_pod": {
        # 元数据
        "name": "重启 Pod",
        "category": "availability",
        "target_type": "Pod",
        "risk_level": "medium",
        "requires_approval": True,
        "rollback_action_id": None,
        "estimated_duration_seconds": 60,
        "description": "对目标 Pod 执行 kubectl delete pod,触发 ReplicaSet 自动重新调度。",
        "input_schema": {
            "type": "object",
            "properties": {
                "graceful": {"type": "boolean", "default": True},
                "grace_period_seconds": {"type": "integer", "default": 30, "minimum": 0, "maximum": 300},
            },
        },
        # 影响传播
        "propagation": [
            {"edge": "ROUTES_TO", "direction": "reverse", "max_depth": 1,
             "target_type": "Service", "impact": "low",
             "note": "Service Endpoints 临时少 1 个就绪 Pod"},
            {"edge": "CONTAINS", "direction": "reverse", "max_depth": 1,
             "target_type": "Deployment", "impact": "minimal",
             "note": "ReplicaSet 自动重新调度新 Pod"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 3,
             "impact": "minimal",
             "note": "向上影响 Component / Application(短暂感知)"},
        ],
        "sla_impact_estimate": "< 0.1%",
        "warnings": ["该 Pod 提供的服务在 30-60 秒内不可用",
                     "若 Pod 是 Deployment 唯一副本(replicas=1),将引发短暂服务中断"],
    },

    "scale_deployment": {
        "name": "调整 Deployment 副本",
        "category": "scale",
        "target_type": "Deployment",
        "risk_level": "low",
        "requires_approval": False,
        "rollback_action_id": "scale_deployment",
        "estimated_duration_seconds": 90,
        "description": "对 Deployment 增减副本数。正数扩容,负数缩容。",
        "input_schema": {
            "type": "object",
            "properties": {
                "replicas_delta": {"type": "integer", "default": 1, "minimum": -10, "maximum": 10},
            },
            "required": ["replicas_delta"],
        },
        "propagation": [
            {"edge": "CONTAINS", "direction": "forward", "max_depth": 1,
             "target_type": "Pod", "impact": "minimal",
             "note": "新增/减少 Pod 副本"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 2,
             "impact": "minimal",
             "note": "Component 承载能力变化"},
        ],
        "sla_impact_estimate": "< 0.1%",
        "warnings": ["扩容后成本会增加,建议业务低峰期再缩容",
                     "缩容到 < 期望副本数会触发 Pod 删除,影响在 Pod 上的连接"],
    },

    "rollback_deployment": {
        "name": "回滚 Deployment 版本",
        "category": "rollback",
        "target_type": "Deployment",
        "risk_level": "high",
        "requires_approval": True,
        "rollback_action_id": "rollback_deployment",
        "estimated_duration_seconds": 180,
        "description": "kubectl rollout undo,把 Deployment 回退到上一版本(或指定 revision)。",
        "input_schema": {
            "type": "object",
            "properties": {
                "revision": {"type": "integer", "minimum": 1},
            },
        },
        "propagation": [
            {"edge": "CONTAINS", "direction": "forward", "max_depth": 1,
             "target_type": "Pod", "impact": "medium",
             "note": "所有 Pod 滚动重启"},
            {"edge": "ROUTES_TO", "direction": "reverse", "max_depth": 2,
             "target_type": "Service", "impact": "medium",
             "note": "滚动期间 Service 部分 Endpoints 切换"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 2,
             "impact": "medium",
             "note": "Component / Application 部分流量回退"},
        ],
        "sla_impact_estimate": "0.5% - 2%",
        "warnings": ["滚动重启期间部分实例不可用,持续 1-3 分钟",
                     "回滚到旧版可能引入已知 bug",
                     "若 ConfigMap 已升级,旧版本可能与新配置不兼容"],
    },

    "refresh_secret": {
        "name": "刷新 Secret",
        "category": "config",
        "target_type": "Secret",
        "risk_level": "medium",
        "requires_approval": True,
        "rollback_action_id": None,
        "estimated_duration_seconds": 300,
        "description": "更新 Secret 内容并(可选)滚动重启所有引用它的 Pod。",
        "input_schema": {
            "type": "object",
            "properties": {
                "trigger_pod_restart": {"type": "boolean", "default": True},
            },
        },
        "propagation": [
            {"edge": "USES", "direction": "reverse", "max_depth": 2,
             "target_type": "Pod", "impact": "medium",
             "note": "所有引用此 Secret 的 Pod 滚动重启"},
            {"edge": "USES", "direction": "reverse", "max_depth": 1,
             "target_type": "Deployment", "impact": "low",
             "note": "Deployment 触发滚动更新"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 3,
             "impact": "low",
             "note": "Component / Application 滚动期间 SLA 短暂影响"},
        ],
        "sla_impact_estimate": "0.1% - 0.5%",
        "warnings": ["旧 Secret 一旦覆盖无法回滚,执行前应备份",
                     "若新 Secret 内容错误,所有引用 Pod 会启动失败"],
    },

    "drain_node": {
        "name": "驱逐 Node 上的 Pod",
        "category": "drain",
        "target_type": "KubernetesNode",
        "risk_level": "high",
        "requires_approval": True,
        "rollback_action_id": None,
        "estimated_duration_seconds": 600,
        "description": "对 Node 执行 cordon + drain,将其上 Pod 迁移到其他节点。",
        "input_schema": {
            "type": "object",
            "properties": {
                "ignore_daemonsets": {"type": "boolean", "default": True},
                "delete_local_data": {"type": "boolean", "default": False},
                "force": {"type": "boolean", "default": False},
            },
        },
        "propagation": [
            {"edge": "SCHEDULED_ON", "direction": "reverse", "max_depth": 1,
             "target_type": "Pod", "impact": "high",
             "note": "节点上所有 Pod 被驱逐重新调度"},
            {"edge": "CONTAINS", "direction": "reverse", "max_depth": 2,
             "target_type": "Deployment", "impact": "medium",
             "note": "受影响 Pod 所属 Deployment 触发重新调度"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 3,
             "impact": "medium",
             "note": "受影响应用短暂部分实例不可用"},
        ],
        "sla_impact_estimate": "1% - 5%",
        "warnings": ["节点上所有 Pod 不可用 5-10 分钟",
                     "若集群资源紧张,Pod 重新调度可能失败",
                     "DaemonSet Pod 默认保留(ignore_daemonsets=True)"],
    },

    "kill_query": {
        "name": "终止 MySQL 慢查询",
        "category": "other",
        "target_type": "MySQL",
        "risk_level": "medium",
        "requires_approval": False,
        "rollback_action_id": None,
        "estimated_duration_seconds": 5,
        "description": "对 MySQL 执行 KILL QUERY,终止特定连接的当前 SQL。",
        "input_schema": {
            "type": "object",
            "properties": {
                "query_id": {"type": "string"},
                "min_duration_seconds": {"type": "integer", "default": 30},
            },
            "required": ["query_id"],
        },
        "propagation": [
            {"edge": "USES", "direction": "reverse", "max_depth": 2,
             "target_type": "Pod", "impact": "low",
             "note": "依赖此 MySQL 的 Pod 该查询失败,客户端需重试"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 3,
             "impact": "low",
             "note": "上游应用收到查询失败响应"},
        ],
        "sla_impact_estimate": "0.01% - 0.1%",
        "warnings": ["被杀 SQL 已执行的部分会回滚(如果在事务里)",
                     "应用需具备重试能力,否则用户感知"],
    },

    "restart_service": {
        "name": "重启 Service Endpoints",
        "category": "availability",
        "target_type": "Service",
        "risk_level": "low",
        "requires_approval": False,
        "rollback_action_id": None,
        "estimated_duration_seconds": 30,
        "description": "重新生成 Service Endpoints,触发 kube-proxy 同步 iptables。",
        "input_schema": {
            "type": "object",
            "properties": {
                "drop_idle_seconds": {"type": "integer", "default": 0},
            },
        },
        "propagation": [
            {"edge": "ROUTES_TO", "direction": "forward", "max_depth": 1,
             "target_type": "Pod", "impact": "minimal",
             "note": "Endpoints 重新生成,Pod 不动"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 3,
             "impact": "minimal",
             "note": "应用层无感"},
        ],
        "sla_impact_estimate": "< 0.05%",
        "warnings": ["重启期间(< 5 秒)新建连接可能短暂失败"],
    },

    "clear_cache": {
        "name": "清空 Redis 缓存",
        "category": "other",
        "target_type": "Redis",
        "risk_level": "medium",
        "requires_approval": True,
        "rollback_action_id": None,
        "estimated_duration_seconds": 60,
        "description": "对 Redis 执行 FLUSHDB / SCAN+DEL,清除指定范围缓存。",
        "input_schema": {
            "type": "object",
            "properties": {
                "scope": {"type": "string", "enum": ["all", "db", "pattern"], "default": "pattern"},
                "db_index": {"type": "integer", "default": 0, "minimum": 0, "maximum": 15},
                "key_pattern": {"type": "string"},
            },
        },
        "propagation": [
            {"edge": "USES", "direction": "reverse", "max_depth": 2,
             "target_type": "Pod", "impact": "high",
             "note": "依赖此 Redis 的 Pod 缓存击穿,负载暴增"},
            {"edge": "USES", "direction": "reverse", "max_depth": 1,
             "target_type": "MySQL", "impact": "high",
             "note": "上游 DB 在缓存击穿后承担直接负载"},
            {"edge": "BELONGS_TO", "direction": "forward", "max_depth": 3,
             "impact": "medium",
             "note": "应用响应延迟显著增加,可能引发雪崩"},
        ],
        "sla_impact_estimate": "1% - 10%",
        "warnings": ["缓存击穿会让 DB 在短时间内承担数倍负载",
                     "scope=all/db 风险极高,生产环境建议只用 pattern",
                     "建议预热(主动 warm-up)而非裸清空"],
    },
}


def get_action(action_id: str) -> dict | None:
    """获取动作模板。返回 None 表示不存在。"""
    return ACTION_DEFS.get(action_id)


def list_actions(target_type: str | None = None,
                 category: str | None = None,
                 risk_level: str | None = None) -> list[dict]:
    """列动作,可按目标类型 / 类别 / 风险级别过滤。"""
    actions = []
    for action_id, defn in ACTION_DEFS.items():
        if target_type and defn["target_type"] != target_type:
            continue
        if category and defn["category"] != category:
            continue
        if risk_level and defn["risk_level"] != risk_level:
            continue
        actions.append({"action_id": action_id, **defn})
    return actions


# ============================================================
# Finding -> Action 推荐映射(基于 InspectionRule)
# 与 scripts/generate_recovery_actions.py 中的 RULE_ACTION_MAP 保持同步
# ============================================================

RULE_ACTION_SUGGESTIONS: dict[str, list[tuple[str, str, float]]] = {
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


def suggest_for_rule(rule_id: str) -> list[dict]:
    """给定 InspectionRule.rule_id,返回推荐动作列表。"""
    suggestions = RULE_ACTION_SUGGESTIONS.get(rule_id, [])
    return [
        {
            "action_id": action_id,
            "rationale": rationale,
            "confidence": confidence,
            **(ACTION_DEFS.get(action_id) or {}),
        }
        for action_id, rationale, confidence in suggestions
    ]
