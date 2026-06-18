"""Recovery Action API — PRD-001 Sprint 1。

Sprint 1 范围:
- GET  /api/v1/recovery/actions               列动作模板(可过滤)
- GET  /api/v1/recovery/actions/{action_id}   单个动作详情
- GET  /api/v1/recovery/suggestions           基于 InspectionRule 推荐动作
- POST /api/v1/recovery/dry-run               影响范围预演

Sprint 2 才做(暂未实现):
- POST /api/v1/recovery/execute
- GET  /api/v1/recovery/executions/{id}
- POST /api/v1/recovery/approval/{id}/approve|reject
- POST /api/v1/recovery/executions/{id}/rollback
- GET  /api/v1/recovery/history
"""

from typing import Optional
from fastapi import APIRouter, HTTPException, Query
from pydantic import BaseModel, Field

from app.recovery.action_defs import ACTION_DEFS, get_action, list_actions, suggest_for_rule
from app.recovery.cascade import dry_run as compute_dry_run

router = APIRouter(prefix="/api/v1/recovery", tags=["Recovery"])


# ============================================================
# Pydantic 模型
# ============================================================

class DryRunRequest(BaseModel):
    action_id: str = Field(..., description="动作 ID,如 scale_deployment")
    target_resource_id: str = Field(..., description="目标资源的 node_id")
    input_params: Optional[dict] = Field(default_factory=dict, description="动作输入参数")
    finding_id: Optional[str] = Field(None, description="可选,触发动作的 InspectionFinding")


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
