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

