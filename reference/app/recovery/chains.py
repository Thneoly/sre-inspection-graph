"""Recovery Chain — PRD-001 Phase 2 余项(动作链 / 编排器)。

声明式多步恢复编排。从 CHAIN_TEMPLATES 选 template,链启动时:
1. 计算最高风险步,若 medium/high → 整链产生一次链级 ApprovalRequest;low 直接 executing
2. 顺序跑每个 step → 每步用 `execute()` 创建普通 RecoveryExecution(chain_id 标记 + chain_step_index)
   - step 内部 verify=verify_required;auto_rollback=False(失败由链级 on_failure 决定)
3. step 失败 / verify_failed → 按 on_failure 处理:
   - stop:chain.status=partial,停
   - rollback_all:反向逐个 rollback 前置成功 step → chain.status=rolled_back
   - continue:继续 N+1

设计要点:
- chain 是 sync 串行(Phase 2 仅串行,并行留 Phase 3)
- chain 级审批:任一步 requires_approval / 非 low → 整链一次审批(approver_team 取风险最高)
  审批 reject → chain.status=failed,无 step 执行
  审批 approve → 自动 _run_chain_steps
- step execution.chain_id / chain_step_index 反向关联,前端可展示 timeline
- verify 单步:step.verify_required=True → step execution.verify=True(verifier 跑);
  verify_failed 即视为 step 失败,触发 on_failure。区别于普通执行的"verify_failed → auto rollback":
  在 chain 内 verify_failed 由 chain on_failure 接管,step 自身不要 auto_rollback(避免双重回滚)
"""
from __future__ import annotations

import logging
import uuid
from datetime import datetime, timezone

from app.datasource.models import RecoveryChain, RecoveryExecution
from app.datasource.store import store
from app.recovery.action_defs import get_action, get_chain_template
from app.recovery.execution import (
    ExecutionError,
    _derive_cluster_id,
    _do_rollback,
    _run_handler_and_persist,
)
from app.recovery.cascade import dry_run as compute_dry_run

logger = logging.getLogger(__name__)


# ============================================================
# 链发起
# ============================================================

def execute_chain(template_id: str,
                  target_resource_id: str,
                  initiated_by: str = "system",
                  on_failure_override: str | None = None,
                  request_reason: str = "") -> RecoveryChain:
    """根据 chain template 发起一条 chain。

    返回 RecoveryChain:
    - 若整链无 requires_approval / 全 low → 直接 _run_chain_steps,返回 final status
    - 否则 → 创建链级 ApprovalRequest,返回 status=pending_approval(awaiting),
      后续审批通过 → _continue_chain_after_approval 跑完

    抛 ExecutionError 表示前置校验失败(template 不存在 / target 不匹配 target_type)。
    """
    template = get_chain_template(template_id)
    if template is None:
        raise ExecutionError(f"unknown chain template: {template_id}", 404)

    steps = template.get("steps", [])
    if not steps:
        raise ExecutionError(f"chain template '{template_id}' has no steps", 400)

    # 校验 target 类型与第一步 action.target_type 匹配
    first_action_id = steps[0]["action_id"]
    first_action = get_action(first_action_id)
    if first_action is None:
        raise ExecutionError(f"chain step references unknown action: {first_action_id}", 400)
    target_node = store.get_node(target_resource_id)
    if target_node and target_node.type != first_action["target_type"]:
        raise ExecutionError(
            f"target type {target_node.type} mismatches first step expected {first_action['target_type']}",
            code=400,
        )

    now_iso = datetime.now(timezone.utc).isoformat()
    chain = RecoveryChain(
        chain_id=str(uuid.uuid4()),
        template_id=template_id,
        target_resource_id=target_resource_id,
        status="pending",
        on_failure=on_failure_override or template.get("on_failure", "stop"),
        total_steps=len(steps),
        initiated_by=initiated_by,
        initiated_at=now_iso,
        template_name=template.get("name", template_id),
    )
    store.add_chain(chain)

    # 链级审批:任一步 risk_level != low 或 requires_approval=True → 整链审批
    needs_approval = False
    max_risk = "low"
    for step in steps:
        ad = get_action(step["action_id"]) or {}
        if ad.get("risk_level") != "low" or ad.get("requires_approval"):
            needs_approval = True
            cur_risk = ad.get("risk_level", "low")
            if cur_risk == "high" or (cur_risk == "medium" and max_risk == "low"):
                max_risk = cur_risk

    if needs_approval:
        # 创建链级审批(approver_team 派生用最高风险 step 的 target)
        from app.recovery.approval import _derive_approver_team
        from app.datasource.models import ApprovalRequest
        from datetime import timedelta

        approver_team = _derive_approver_team(target_resource_id)
        approval = ApprovalRequest(
            approval_id=str(uuid.uuid4()),
            execution_id=chain.chain_id,           # 链级审批用 chain_id 占 execution_id 字段
            requested_by=initiated_by,
            requested_at=now_iso,
            request_reason=request_reason or f"chain '{template.get('name', template_id)}' on {target_resource_id}",
            approval_status="pending",
            expiry_at=(datetime.now(timezone.utc) + timedelta(hours=24)).isoformat(),
            approver_team=approver_team,
        )
        store.add_approval(approval)
        chain.approval_id = approval.approval_id
        chain.status = "awaiting_approval"
        chain.failure_reason = f"chain-level approval required (max risk={max_risk})"
        store.update_chain(chain)
        return chain

    # 全 low → 直接跑
    chain.status = "executing"
    store.update_chain(chain)
    return _run_chain_steps(chain)


def continue_chain_after_approval(chain_id: str) -> RecoveryChain:
    """链级审批通过后调,跑完整链。"""
    chain = store.get_chain(chain_id)
    if chain is None:
        raise ExecutionError(f"chain not found: {chain_id}", 404)
    if chain.status != "awaiting_approval":
        raise ExecutionError(
            f"chain status is {chain.status}, expected awaiting_approval",
            code=409,
        )
    chain.status = "executing"
    chain.failure_reason = ""
    store.update_chain(chain)
    return _run_chain_steps(chain)


def abort_chain(chain_id: str, reason: str = "") -> RecoveryChain:
    """中止运行中的 chain(标 aborted,不做反向 rollback)。

    仅允许 pending / awaiting_approval / executing 状态。已 succeeded / partial /
    rolled_back / failed / aborted 不再可中止。
    """
    chain = store.get_chain(chain_id)
    if chain is None:
        raise ExecutionError(f"chain not found: {chain_id}", 404)
    if chain.status not in ("pending", "awaiting_approval", "executing"):
        raise ExecutionError(
            f"chain status is {chain.status}, cannot abort",
            code=409,
        )
    chain.status = "aborted"
    chain.completed_at = datetime.now(timezone.utc).isoformat()
    chain.failure_reason = reason or "aborted by user"
    store.update_chain(chain)
    return chain


# ============================================================
# 内部:实际跑步骤
# ============================================================

def _run_chain_steps(chain: RecoveryChain) -> RecoveryChain:
    """从 chain.current_step_index 开始顺序跑,直到完成或触发 on_failure。"""
    template = get_chain_template(chain.template_id)
    if template is None:
        chain.status = "failed"
        chain.failure_reason = f"template disappeared: {chain.template_id}"
        chain.completed_at = datetime.now(timezone.utc).isoformat()
        store.update_chain(chain)
        return chain

    steps = template["steps"]
    had_failure = False

    while chain.current_step_index < len(steps):
        idx = chain.current_step_index
        step = steps[idx]
        ex = _run_single_step(chain, idx, step)
        chain.step_executions.append(ex.execution_id)
        chain.current_step_index = idx + 1
        store.update_chain(chain)

        step_ok = ex.status == "succeeded" and (
            not step.get("verify_required", True)
            or ex.verify_status in ("passed", "skipped", "not_supported", "")
        )
        if not step_ok:
            had_failure = True
            if chain.on_failure == "continue":
                # 记下失败原因后继续下一步
                reason = (ex.result or {}).get("error") or f"verify_status={ex.verify_status}"
                chain.failure_reason = (chain.failure_reason + " | " if chain.failure_reason else "") + \
                    f"step {idx} ({ex.action_id}) failed: {reason}"
                store.update_chain(chain)
                continue
            # stop / rollback_all → 终止链
            return _handle_step_failure(chain, idx, ex)

    chain.status = "partial" if had_failure else "succeeded"
    chain.completed_at = datetime.now(timezone.utc).isoformat()
    store.update_chain(chain)
    return chain


def _handle_step_failure(chain: RecoveryChain, failed_idx: int,
                          failed_ex: RecoveryExecution) -> RecoveryChain:
    """处理 stop / rollback_all(continue 已在 _run_chain_steps 内部处理)。"""
    reason = (failed_ex.result or {}).get("error") or f"verify_status={failed_ex.verify_status}"
    chain.failure_reason = f"step {failed_idx} ({failed_ex.action_id}) failed: {reason}"

    if chain.on_failure == "rollback_all":
        rolled = []
        for prev_eid in reversed(chain.step_executions[:-1]):  # 排除当前失败 step 自身
            prev_ex = store.get_execution(prev_eid)
            if prev_ex is None or prev_ex.status != "succeeded":
                continue
            try:
                rb = _do_rollback(
                    original=prev_ex,
                    initiated_by="chain-rollback_all",
                    reason=f"chain {chain.chain_id} rollback_all triggered by step {failed_idx}",
                    auto_rollback_marker=True,
                )
                rolled.append(rb.execution_id)
            except Exception as e:  # noqa: BLE001
                logger.warning("chain rollback_all step %s failed: %s", prev_eid, e)
        chain.status = "rolled_back"
        chain.completed_at = datetime.now(timezone.utc).isoformat()
        chain.failure_reason += f" | rolled back {len(rolled)} prior step(s)"
        store.update_chain(chain)
        return chain

    # 默认 stop
    chain.status = "partial"
    chain.completed_at = datetime.now(timezone.utc).isoformat()
    store.update_chain(chain)
    return chain


def _run_single_step(chain: RecoveryChain, idx: int, step: dict) -> RecoveryExecution:
    action_id = step["action_id"]
    action = get_action(action_id)
    target_id = chain.target_resource_id  # Phase 2 全部从 input 取(target_from=input)
    params = dict(step.get("params") or {})

    # 跑 dry_run 校验
    dry_result = compute_dry_run(action_id, target_id, params)

    now_iso = datetime.now(timezone.utc).isoformat()
    target_node = store.get_node(target_id)

    ex = RecoveryExecution(
        execution_id=str(uuid.uuid4()),
        action_id=action_id,
        target_resource_id=target_id,
        target_resource_type=target_node.type if target_node else (action or {}).get("target_type", ""),
        finding_id=None,
        input_params=params,
        dry_run_result=dry_result,
        status="executing",
        initiated_by=chain.initiated_by,
        request_reason=f"chain {chain.chain_id} step {idx}",
        initiated_at=now_iso,
        executed_at=now_iso,
        cluster_id=_derive_cluster_id(target_id, target_node),
        chain_id=chain.chain_id,
        chain_step_index=idx,
    )
    store.add_execution(ex)

    verify = bool(step.get("verify_required", True))
    return _run_handler_and_persist(ex, auto_rollback=False, verify=verify)


# ============================================================
# 列表 / 查询
# ============================================================

def list_chains(status: str | None = None) -> list[RecoveryChain]:
    """按 status 过滤列表;新→旧排序。"""
    return store.list_chains(status=status)


def get_chain(chain_id: str) -> RecoveryChain | None:
    return store.get_chain(chain_id)
