"""DSS 数据模型"""
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Optional


@dataclass
class DataNode:
    id: str
    type: str
    name: str
    properties: dict[str, Any] = field(default_factory=dict)


@dataclass
class DataEdge:
    id: str
    source_id: str
    target_id: str
    relationship_type: str
    relationship_name: str = ""
    properties: dict[str, Any] = field(default_factory=dict)


@dataclass
class MetricSnapshot:
    snapshot_id: str
    resource_id: str
    metric_name: str
    current_value: float
    unit: str = "percent"
    fetched_at: str = ""
    warning_breached: bool = False
    critical_breached: bool = False


@dataclass
class FaultStage:
    sequence: int
    offset_seconds: int
    health: str
    risk: str
    metric_name: str = ""
    metric_value: float = 0.0
    unit: str = "percent"
    triggers_alert: bool = False
    triggers_finding: bool = False


@dataclass
class FaultInjection:
    injection_id: str
    fault_type: str
    target_id: str
    current_stage: int = 0
    total_stages: int = 0
    status: str = "injected"
    injected_at: str = ""
    stages: list[FaultStage] = field(default_factory=list)


# ============================================================
# Recovery Action Engine (PRD-001)
# ============================================================

@dataclass
class RecoveryAction:
    """恢复动作模板 — 配置对象,启动时从 Neo4j 加载到内存。

    一个 RecoveryAction 描述"对某种资源类型可以做什么"——
    比如 restart_pod 描述"对 Pod 可以重启"。模板本身不绑定具体资源实例,
    实际执行时由 RecoveryExecution 关联到 target_resource_id。
    """
    action_id: str                 # restart_pod, scale_deployment, ...
    action_name: str               # 中文名: 重启 Pod
    action_category: str           # availability | config | scale | rollback | drain | other
    target_resource_type: str      # Pod, Deployment, Secret, KubernetesNode, MySQL, Redis, Service
    risk_level: str                # low | medium | high
    requires_approval: bool = False
    rollback_action_id: Optional[str] = None    # 反向动作的 action_id,无回滚为 None
    input_schema: dict[str, Any] = field(default_factory=dict)  # JSON Schema 描述输入参数
    description: str = ""
    estimated_duration_seconds: int = 60
    dry_run_handler: str = ""      # 模块路径: app.recovery.handlers.scale_deployment_dry_run
    execute_handler: str = ""      # 模块路径: app.recovery.handlers.scale_deployment_execute


@dataclass
class RecoveryExecution:
    """一次恢复动作的执行实例 — 事件对象,记录"对哪个资源做了什么"。

    生命周期:
        pending → dry_run_ok → awaiting_approval → approved/rejected
                                                    ↓
                                                 executing → succeeded/failed
                                                                  ↓
                                                              rolled_back (可选)
    """
    execution_id: str              # uuid
    action_id: str                 # 引用 RecoveryAction.action_id
    target_resource_id: str        # 实际目标资源
    target_resource_type: str
    finding_id: Optional[str] = None    # 触发动作的 InspectionFinding(可空)
    input_params: dict[str, Any] = field(default_factory=dict)
    dry_run_result: dict[str, Any] = field(default_factory=dict)
    status: str = "pending"
    initiated_by: str = ""
    approval_id: Optional[str] = None
    request_reason: str = ""
    initiated_at: str = ""
    executed_at: str = ""
    completed_at: str = ""
    result: dict[str, Any] = field(default_factory=dict)
    rollback_execution_id: Optional[str] = None
    reverses_execution_id: Optional[str] = None  # 若本 execution 是回滚动作,指向被回滚的原 execution
    # Phase 2 余项 — 跨集群恢复编排 / 自动验证 / 动作链
    cluster_id: str = ""                  # 目标所属集群(从 target.cluster_id 或 target_id 第二段解析)
    verify_status: str = ""               # "" | passed | failed | skipped | not_supported | timeout
    verify_result: dict[str, Any] = field(default_factory=dict)
    verified_at: str = ""
    chain_id: str = ""                    # 若属于某 RecoveryChain
    chain_step_index: int = -1            # 在 chain 中的步骤索引,-1 = 非链步骤


@dataclass
class ChangeEvent:
    """变更事件 — 事件对象,记录"什么时间什么资源被谁怎么改了"。

    PRD-002 Sprint 1。4 类 change_type:
        configmap_updated / secret_rotated / deployment_rolled / image_pushed

    propagated_to 是写入时一次性算好缓存的影响范围(沿强依赖关系反向 BFS),
    供 /correlated 查询 O(1) 命中。target 不在 DSS 时仍可记录(propagated_to=[])。
    """
    change_event_id: str
    change_type: str                                  # configmap_updated | secret_rotated |
                                                       # deployment_rolled | image_pushed
    target_resource_id: str
    target_resource_type: str
    changed_at: str                                   # ISO8601
    changed_by: str = ""
    source: str = "manual"                            # k8s_api | argo_cd | gitops | manual | unknown
                                                       # | flagd
    description: str = ""
    diff_summary: dict[str, Any] = field(default_factory=dict)
    related_commit: str = ""
    related_pr: str = ""
    severity_estimate: str = "low"                    # low | medium | high
    propagated_to: list[str] = field(default_factory=list)
    # Phase 2 — Git/CI 关联 + 集群来源 + 结构化 YAML diff
    commit_sha: str = ""                              # Git commit hash(规范字段,优先于 related_commit)
    pipeline_url: str = ""                            # CI pipeline 运行链接
    git_repo: str = ""                                # 仓库 URL
    cluster_id: str = ""                              # 来源集群(watcher / webhook 填)
    yaml_diff: str = ""                               # unified diff 文本(yaml_diff.compute_yaml_diff 产出)


@dataclass
class AlertRule:
    """告警规则 — PRD-004 Phase 2。

    从 health_rules 的 QueryDef 阈值生成。一条 rule 描述"某指标超某阈值就告警"。
    connector 检测到 critical breach 时,按 rule 产出 AlertEvent。

    与 legacy simulation.py 的 FAULT_TYPES alert_rule 字符串不同 —— 这是结构化的、
    可查询的规则对象,挂在 DSS store。
    """
    rule_id: str                                      # 形如 alert_rule:span_p99_ms:critical
    metric_name: str                                  # 对应 QueryDef.name
    severity: str = "critical"                        # warning | critical
    threshold: float = 0.0                            # 触发阈值(>=)
    direction: str = "high"                           # high | low — 哪个方向算差
    unit: str = ""                                    # 指标单位
    description: str = ""                             # 人读描述
    enabled: bool = True


@dataclass
class AlertEvent:
    """告警事件 — PRD-004 Phase 2。

    connector(目前 prometheus)检测到 critical breach 时产出。镜像 ChangeEvent 的
    DSS 主存储 + Neo4j dual-write 模式,使 PRD-002 Phase 2 的 correlate_alerts 可
    从 DSS 读(不再只依赖 Neo4j)。

    AlertEvent 落地后,record_change 会自动 correlate_and_persist 关联窗口内变更
    (CORRELATED_WITH 边),形成 "变更 → 告警" 的双向可查链路。
    """
    alert_event_id: str
    alert_name: str                                   # 规则名 / 告警名
    severity: str = "critical"                        # warning | critical
    status: str = "firing"                            # firing | resolved
    fired_at: str = ""                                # ISO8601(record_alert 填)
    resource_ref: str = ""                            # 被告警资源 DSS node_id
    rule_id: str = ""                                 # 触发的 AlertRule
    metric_name: str = ""                             # 触发指标
    metric_value: float = 0.0                         # 触发时的值
    summary: str = ""
    description: str = ""
    cluster_id: str = ""
    resolved_at: str = ""


@dataclass
class ApprovalRequest:
    """审批请求 — 事件对象,只在 high_risk / requires_approval 动作下创建。

    生命周期: pending → approved | rejected | expired
    默认有效期 24 小时,过期需重新发起。
    """
    approval_id: str
    execution_id: str
    requested_by: str
    requested_at: str
    request_reason: str = ""
    approver_id: str = ""
    approval_status: str = "pending"        # pending | approved | rejected | expired
    approved_at: str = ""
    approval_comment: str = ""
    expiry_at: str = ""
    approver_team: str = ""  # 负责审批的团队(从 target.owner_team 派生),软记录用

