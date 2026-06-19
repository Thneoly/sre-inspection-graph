"""Approval Flow — PRD-001 Sprint 3.

负责管理 ApprovalRequest 生命周期:
    pending → approved → 触发 _continue_after_approval → execution succeeds/fails
            → rejected
            → expired (24h 未操作,读时检查)

设计要点:
- 不强制 RBAC:approver_id 任填,只做审计
- approver_team 从 target.owner_team 派生(Pod/Service 等沿 BELONGS_TO 上溯)
- approve 端点同步触发执行(一次 HTTP 调用完成审批 + 执行)
- 24h 过期改为读时检查,不起后台 cron
"""

import uuid
from datetime import datetime, timedelta, timezone
from typing import Optional

from app.datasource.models import ApprovalRequest, RecoveryExecution
from app.datasource.store import store

APPROVAL_TTL_HOURS = 24
DEFAULT_APPROVER_TEAM = "platform"


class ApprovalError(Exception):
    """审批流异常。message 直接给用户看。"""
    def __init__(self, message: str, code: int = 400):
        self.message = message
        self.code = code
        super().__init__(message)


# ============================================================
# Public API
# ============================================================

def request_approval(execution: RecoveryExecution,
                     requested_by: str,
                     request_reason: str = "") -> ApprovalRequest:
    """对一个 awaiting_approval 状态的 execution 创建审批请求。

    调用方负责把 execution.status 提前改成 awaiting_approval 并 update_execution。
    """
    now = datetime.now(timezone.utc)
    approval = ApprovalRequest(
        approval_id=str(uuid.uuid4()),
        execution_id=execution.execution_id,
        requested_by=requested_by,
        requested_at=now.isoformat(),
        request_reason=request_reason,
        approval_status="pending",
        expiry_at=(now + timedelta(hours=APPROVAL_TTL_HOURS)).isoformat(),
        approver_team=_derive_approver_team(execution.target_resource_id),
    )
    store.add_approval(approval)
    return approval


def approve(approval_id: str,
            approver_id: str,
            comment: str = "") -> tuple[ApprovalRequest, RecoveryExecution]:
    """批准一个 pending approval,并立即触发关联 execution 完成。

    返回 (approval, execution) — execution.status 应为 succeeded 或 failed。

    幂等性:重复 approve 同一 ID 抛 409。
    并发安全:状态从 pending 改成非 pending 是单步内存写,Python GIL 保证原子。
    """
    approval = _get_or_expire(approval_id)
    if approval is None:
        raise ApprovalError(f"approval not found: {approval_id}", 404)
    if approval.approval_status != "pending":
        raise ApprovalError(
            f"approval is {approval.approval_status}, cannot approve", 409,
        )

    now = datetime.now(timezone.utc).isoformat()
    approval.approval_status = "approved"
    approval.approver_id = approver_id
    approval.approved_at = now
    approval.approval_comment = comment
    store.update_approval(approval)

    # 触发后续执行(circular import — 延迟导入)
    from app.recovery.execution import _continue_after_approval

    execution = _continue_after_approval(approval.execution_id)
    return approval, execution


def reject(approval_id: str,
           approver_id: str,
           comment: str = "") -> tuple[ApprovalRequest, RecoveryExecution]:
    """驳回一个 pending approval。execution.status 改为 rejected。"""
    approval = _get_or_expire(approval_id)
    if approval is None:
        raise ApprovalError(f"approval not found: {approval_id}", 404)
    if approval.approval_status != "pending":
        raise ApprovalError(
            f"approval is {approval.approval_status}, cannot reject", 409,
        )

    now = datetime.now(timezone.utc).isoformat()
    approval.approval_status = "rejected"
    approval.approver_id = approver_id
    approval.approved_at = now
    approval.approval_comment = comment
    store.update_approval(approval)

    # 同步把 execution 标 rejected(不进 executing)
    execution = store.get_execution(approval.execution_id)
    if execution is not None:
        execution.status = "rejected"
        execution.completed_at = now
        execution.result = {
            "success": False,
            "error": f"rejected by {approver_id}: {comment}" if comment else f"rejected by {approver_id}",
        }
        store.update_execution(execution)

    return approval, execution


def list_approvals(status: Optional[str] = None) -> list[ApprovalRequest]:
    """列审批请求(按 status 过滤)。读时顺手把过期 pending 标为 expired。"""
    # 先扫一遍把所有过期 pending 标为 expired
    now = datetime.now(timezone.utc)
    for ap in list(store.approvals.values()):
        if ap.approval_status == "pending" and _is_expired(ap, now):
            ap.approval_status = "expired"
            store.update_approval(ap)

    return store.get_approvals_by_status(status)


def get_approval(approval_id: str) -> Optional[ApprovalRequest]:
    """读单条审批,顺手做过期检查。"""
    return _get_or_expire(approval_id)


def is_expired(approval: ApprovalRequest) -> bool:
    """是否已过 TTL 但 status 仍为 pending。"""
    return _is_expired(approval, datetime.now(timezone.utc))


# ============================================================
# Internals
# ============================================================

def _get_or_expire(approval_id: str) -> Optional[ApprovalRequest]:
    """取审批,顺手把过期 pending 标 expired 后返回。"""
    approval = store.get_approval(approval_id)
    if approval is None:
        return None
    if approval.approval_status == "pending" and _is_expired(approval, datetime.now(timezone.utc)):
        approval.approval_status = "expired"
        store.update_approval(approval)
    return approval


def _is_expired(approval: ApprovalRequest, now: datetime) -> bool:
    if not approval.expiry_at:
        return False
    try:
        expiry = datetime.fromisoformat(approval.expiry_at)
    except ValueError:
        return False
    if expiry.tzinfo is None:
        expiry = expiry.replace(tzinfo=timezone.utc)
    return expiry < now


def _derive_approver_team(target_resource_id: str) -> str:
    """从 target.owner_team 读;Pod 等沿 BELONGS_TO 上溯到 Component / Application。

    遍历最多 5 跳,避免环。最终默认 "platform"。
    """
    visited: set[str] = set()
    current_id: Optional[str] = target_resource_id
    for _ in range(5):
        if current_id is None or current_id in visited:
            break
        visited.add(current_id)
        node = store.get_node(current_id)
        if node is None:
            break
        team = node.properties.get("owner_team")
        if team:
            return team
        # 沿 BELONGS_TO 顺向上溯(source_id == current_id)
        next_id: Optional[str] = None
        for edge in store.get_all_edges():
            if edge.relationship_type != "BELONGS_TO":
                continue
            if edge.source_id == current_id:
                next_id = edge.target_id
                break
        current_id = next_id

    return DEFAULT_APPROVER_TEAM
