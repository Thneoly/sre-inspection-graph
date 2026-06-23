"""Recovery Action API — PRD-001 Sprint 1 + 2 + 3。

Sprint 1(已上线):
- GET  /api/v1/recovery/actions               列动作模板(可过滤)
- GET  /api/v1/recovery/actions/{action_id}   单个动作详情
- GET  /api/v1/recovery/suggestions           基于 InspectionRule 推荐动作
- POST /api/v1/recovery/dry-run               影响范围预演

Sprint 2:
- POST /api/v1/recovery/execute               执行动作(low_risk 同步;medium/high 进审批)
- GET  /api/v1/recovery/executions            执行历史(过滤 + 分页)
- GET  /api/v1/recovery/executions/{id}       单次执行详情

Sprint 3:
- POST /api/v1/recovery/approvals/{id}/approve   审批通过(自动触发执行)
- POST /api/v1/recovery/approvals/{id}/reject    审批驳回
- GET  /api/v1/recovery/approvals                按 status 列审批
- GET  /api/v1/recovery/approvals/{id}           单条审批详情
- POST /api/v1/recovery/executions/{id}/rollback 回滚已成功的执行
"""

from typing import Optional
from fastapi import APIRouter, HTTPException, Query, Response
from pydantic import BaseModel, Field

from app.recovery.action_defs import ACTION_DEFS, get_action, list_actions, suggest_for_rule
from app.recovery.cascade import dry_run as compute_dry_run
from app.recovery.execution import (
    execute as run_execution,
    list_executions,
    rollback as run_rollback,
    ExecutionError,
)
from app.recovery import approval as approval_mod
from app.datasource.store import store
from app.datasource.models import RecoveryExecution, ApprovalRequest

router = APIRouter(prefix="/api/v1/recovery", tags=["Recovery"])


# ============================================================
# Pydantic 模型
# ============================================================

class DryRunRequest(BaseModel):
    action_id: str = Field(..., description="动作 ID,如 scale_deployment")
    target_resource_id: str = Field(..., description="目标资源的 node_id")
    input_params: Optional[dict] = Field(default_factory=dict, description="动作输入参数")
    finding_id: Optional[str] = Field(None, description="可选,触发动作的 InspectionFinding")


class ExecuteRequest(BaseModel):
    action_id: str = Field(..., description="动作 ID")
    target_resource_id: str = Field(..., description="目标资源的 node_id")
    input_params: Optional[dict] = Field(default_factory=dict, description="动作输入参数")
    finding_id: Optional[str] = Field(None, description="触发的 Finding")
    initiated_by: str = Field("system", description="发起人 ID")
    request_reason: str = Field("", description="申请理由(用于审计 + Sprint 3 审批)")


# ============================================================
# 端点
# ============================================================

@router.get("/actions")
def list_recovery_actions(
    target_type: Optional[str] = Query(None, description="按目标资源类型过滤,如 Pod / Deployment"),
    category: Optional[str] = Query(None, description="按类别过滤,如 scale / rollback / availability"),
    risk_level: Optional[str] = Query(None, pattern="^(low|medium|high)$"),
):
    """列出所有 RecoveryAction 模板。"""
    actions = list_actions(target_type=target_type, category=category, risk_level=risk_level)
    return {
        "actions": [_serialize_action(a) for a in actions],
        "total": len(actions),
    }


@router.get("/actions/{action_id}")
def get_recovery_action(action_id: str):
    """获取单个动作详情。"""
    action = get_action(action_id)
    if action is None:
        raise HTTPException(404, f"action not found: {action_id}")
    return _serialize_action({"action_id": action_id, **action})


@router.get("/suggestions")
def get_suggestions(
    rule_id: Optional[str] = Query(None, description="InspectionRule.rule_id"),
    finding_id: Optional[str] = Query(None, description="InspectionFinding.id(暂不支持,留 Sprint 2)"),
):
    """基于 InspectionRule 或 Finding 返回推荐动作。

    Sprint 1 只支持 rule_id 路径(直接读 RULE_ACTION_SUGGESTIONS);
    Sprint 2 支持 finding_id,会查 Neo4j 拿 finding 的 rule_id 后再走同路径。
    """
    if not rule_id and not finding_id:
        raise HTTPException(400, "either rule_id or finding_id required")

    if finding_id and not rule_id:
        raise HTTPException(501, "finding_id lookup not implemented yet (Sprint 2)")

    suggestions = suggest_for_rule(rule_id)
    return {
        "rule_id": rule_id,
        "finding_id": finding_id,
        "suggestions": [_serialize_suggestion(s) for s in suggestions],
        "total": len(suggestions),
    }


@router.post("/dry-run")
def dry_run(req: DryRunRequest):
    """对一个 (动作 + 目标) 计算影响范围。

    返回结构包含:
    - target_valid:目标是否合法(类型不匹配会返 False 但不抛异常)
    - affected_resources:受影响资源列表(类型 + 严重度 + 关系链)
    - estimated_duration_seconds / estimated_sla_impact / warnings
    - rollback_action_id / rollback_input_params(如果可回滚)

    详见 `app.recovery.cascade.dry_run` docstring。
    """
    result = compute_dry_run(
        action_id=req.action_id,
        target_resource_id=req.target_resource_id,
        input_params=req.input_params or {},
    )
    # finding_id 透传到结果(Sprint 2 用于审计追溯)
    if req.finding_id:
        result["finding_id"] = req.finding_id
    return result


# ============================================================
# 序列化辅助
# ============================================================

def _serialize_action(action: dict) -> dict:
    """裁掉内部用的 propagation 规则,只返前端需要的字段。"""
    return {
        "action_id": action["action_id"],
        "action_name": action["name"],
        "action_category": action["category"],
        "target_resource_type": action["target_type"],
        "risk_level": action["risk_level"],
        "requires_approval": action["requires_approval"],
        "rollback_action_id": action.get("rollback_action_id"),
        "estimated_duration_seconds": action["estimated_duration_seconds"],
        "description": action.get("description", ""),
        "input_schema": action.get("input_schema", {}),
        "sla_impact_estimate": action.get("sla_impact_estimate", "n/a"),
        "warnings": list(action.get("warnings", [])),
    }


def _serialize_suggestion(suggestion: dict) -> dict:
    """推荐动作 = 动作信息 + rationale + confidence。"""
    return {
        "action_id": suggestion["action_id"],
        "action_name": suggestion.get("name", ""),
        "rationale": suggestion["rationale"],
        "confidence": suggestion["confidence"],
        "risk_level": suggestion.get("risk_level"),
        "requires_approval": suggestion.get("requires_approval", False),
        "target_resource_type": suggestion.get("target_type"),
    }


# ============================================================
# Sprint 2 端点:execute / executions
# ============================================================

@router.post("/execute")
def execute(req: ExecuteRequest, response: Response):
    """执行恢复动作。

    流程:
    1. 验证 action 存在 + 有 handler
    2. 跑 dry_run 验证目标合法
    3. 创建 RecoveryExecution(uuid)
    4. low_risk + 不需要审批 → 同步 handler → 200 + status=succeeded/failed
       medium / high_risk 或 requires_approval=True → 创建 ApprovalRequest
       → 202 + status=awaiting_approval + approval_id
    """
    try:
        execution = run_execution(
            action_id=req.action_id,
            target_resource_id=req.target_resource_id,
            input_params=req.input_params or {},
            initiated_by=req.initiated_by,
            finding_id=req.finding_id,
            request_reason=req.request_reason,
        )
    except ExecutionError as e:
        raise HTTPException(status_code=e.code, detail=e.message)

    # awaiting_approval → 202 Accepted
    if execution.status == "awaiting_approval":
        response.status_code = 202

    return _serialize_execution(execution)


@router.get("/executions")
def list_recovery_executions(
    status: Optional[str] = Query(None, pattern="^(pending|dry_run_ok|awaiting_approval|approved|rejected|executing|succeeded|failed|rolled_back)$"),
    action_id: Optional[str] = Query(None),
    target_resource_id: Optional[str] = Query(None),
    limit: int = Query(50, ge=1, le=500),
):
    """查询执行历史。"""
    executions = list_executions(
        status=status,
        action_id=action_id,
        target_resource_id=target_resource_id,
        limit=limit,
    )
    return {
        "executions": [_serialize_execution(e) for e in executions],
        "total": len(executions),
    }


@router.get("/executions/{execution_id}")
def get_execution(execution_id: str):
    """获取单次执行详情。"""
    execution = store.get_execution(execution_id)
    if execution is None:
        raise HTTPException(404, f"execution not found: {execution_id}")
    return _serialize_execution(execution)


def _serialize_execution(execution: RecoveryExecution) -> dict:
    """RecoveryExecution → 前端友好 dict。"""
    return {
        "execution_id": execution.execution_id,
        "action_id": execution.action_id,
        "action_name": (get_action(execution.action_id) or {}).get("name", ""),
        "target_resource_id": execution.target_resource_id,
        "target_resource_type": execution.target_resource_type,
        "finding_id": execution.finding_id,
        "input_params": execution.input_params,
        "status": execution.status,
        "initiated_by": execution.initiated_by,
        "request_reason": execution.request_reason,
        "initiated_at": execution.initiated_at,
        "executed_at": execution.executed_at,
        "completed_at": execution.completed_at,
        "result": execution.result,
        "approval_id": execution.approval_id,
        "rollback_execution_id": execution.rollback_execution_id,
        "reverses_execution_id": execution.reverses_execution_id,
        # Phase 2 余项
        "cluster_id": execution.cluster_id,
        "verify_status": execution.verify_status,
        "verify_result": execution.verify_result,
        "verified_at": execution.verified_at,
        "chain_id": execution.chain_id,
        "chain_step_index": execution.chain_step_index,
        # dry_run_result 只在详情接口返回(列表太占空间)
        "dry_run_summary": {
            "affected_count": execution.dry_run_result.get("affected_count", 0),
            "estimated_sla_impact": execution.dry_run_result.get("estimated_sla_impact"),
            "rollback_action_id": execution.dry_run_result.get("rollback_action_id"),
        } if execution.dry_run_result else None,
    }


# ============================================================
# Sprint 3 端点:approvals + rollback
# ============================================================

class ApprovalDecisionRequest(BaseModel):
    approver_id: str = Field(..., description="审批人 ID(任填,只做审计)")
    comment: str = Field("", description="审批意见")


class RollbackRequest(BaseModel):
    initiated_by: str = Field("system", description="发起回滚的用户")
    reason: str = Field("", description="回滚理由")


@router.get("/approvals")
def list_approval_requests(
    status: Optional[str] = Query(
        None, pattern="^(pending|approved|rejected|expired)$",
        description="按状态过滤;默认全部",
    ),
):
    """列出审批请求(读时顺手把过期 pending 标 expired)。"""
    approvals = approval_mod.list_approvals(status=status)
    # 按申请时间倒序
    approvals.sort(key=lambda a: a.requested_at or "", reverse=True)
    return {
        "approvals": [_serialize_approval(a) for a in approvals],
        "total": len(approvals),
    }


@router.get("/approvals/{approval_id}")
def get_approval_detail(approval_id: str):
    """单条审批详情(顺手过期检查)。"""
    approval = approval_mod.get_approval(approval_id)
    if approval is None:
        raise HTTPException(404, f"approval not found: {approval_id}")
    return _serialize_approval(approval)


@router.post("/approvals/{approval_id}/approve")
def approve_approval(approval_id: str, req: ApprovalDecisionRequest):
    """批准审批请求 → 自动触发后续执行。

    返回结构:`{approval, execution}`。execution 应为 succeeded 或 failed。
    """
    try:
        approval, execution = approval_mod.approve(
            approval_id, req.approver_id, req.comment,
        )
    except approval_mod.ApprovalError as e:
        raise HTTPException(status_code=e.code, detail=e.message)
    except ExecutionError as e:
        raise HTTPException(status_code=e.code, detail=e.message)

    return {
        "approval": _serialize_approval(approval),
        "execution": _serialize_execution(execution),
    }


@router.post("/approvals/{approval_id}/reject")
def reject_approval(approval_id: str, req: ApprovalDecisionRequest):
    """驳回审批请求 → execution.status=rejected。"""
    try:
        approval, execution = approval_mod.reject(
            approval_id, req.approver_id, req.comment,
        )
    except approval_mod.ApprovalError as e:
        raise HTTPException(status_code=e.code, detail=e.message)

    return {
        "approval": _serialize_approval(approval),
        "execution": _serialize_execution(execution) if execution else None,
    }


@router.post("/executions/{execution_id}/rollback")
def rollback_execution(execution_id: str, req: RollbackRequest):
    """回滚一个已成功的 execution。创建反向 execution,直接同步执行(不审批)。"""
    try:
        rb_execution = run_rollback(
            execution_id=execution_id,
            initiated_by=req.initiated_by,
            reason=req.reason,
        )
    except ExecutionError as e:
        raise HTTPException(status_code=e.code, detail=e.message)

    return _serialize_execution(rb_execution)


def _serialize_approval(approval: ApprovalRequest) -> dict:
    """ApprovalRequest → 前端友好 dict。"""
    execution = store.get_execution(approval.execution_id)
    return {
        "approval_id": approval.approval_id,
        "execution_id": approval.execution_id,
        "requested_by": approval.requested_by,
        "requested_at": approval.requested_at,
        "request_reason": approval.request_reason,
        "approver_id": approval.approver_id,
        "approver_team": approval.approver_team,
        "approval_status": approval.approval_status,
        "approved_at": approval.approved_at,
        "approval_comment": approval.approval_comment,
        "expiry_at": approval.expiry_at,
        # 嵌入关联 execution 摘要(列表场景前端可直接展示动作名 + 目标)
        "execution_summary": {
            "action_id": execution.action_id,
            "action_name": (get_action(execution.action_id) or {}).get("name", ""),
            "target_resource_id": execution.target_resource_id,
            "target_resource_type": execution.target_resource_type,
            "status": execution.status,
            "dry_run_summary": {
                "affected_count": execution.dry_run_result.get("affected_count", 0),
                "estimated_sla_impact": execution.dry_run_result.get("estimated_sla_impact"),
            } if execution.dry_run_result else None,
        } if execution else None,
    }
