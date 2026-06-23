"""RecoveryExecution Lifecycle 编排 — PRD-001 Sprint 2 + Sprint 3。

Sprint 2 范围:
- low_risk 同步执行(scale_deployment / kill_query / restart_service)

Sprint 3 范围:
- medium / high_risk 进入审批流(awaiting_approval),approve 后回到 executing
- 5 个新 handler:restart_pod / rollback_deployment / refresh_secret / drain_node / clear_cache
- 一键回滚:rollback() 创建反向 execution,直接同步执行(不再二次审批)

执行流程:
                        ┌─ low_risk + no approval ──────────────┐
                        │                                       ↓
    pending → dry_run_ok                                   executing → succeeded
                        │                                       ↓
                        └─ medium/high or requires_approval ─→ failed
                                    ↓
                            awaiting_approval
                                    ↓ (approve / reject)
                            approved / rejected
                                    ↓ (if approved)
                                executing → succeeded → [可选 rollback] → rolled_back
                                                ↓
                                              failed
"""

import uuid
from datetime import datetime, timezone

from app.datasource.models import RecoveryExecution
from app.datasource.store import store
from app.db.neo4j_client import get_driver
from app.recovery.action_defs import get_action
from app.recovery.cascade import dry_run as compute_dry_run
from app.recovery.handlers import get_handler, is_executable


class ExecutionError(Exception):
    """执行流程异常。message 直接给用户看。"""
    def __init__(self, message: str, code: int = 400):
        self.message = message
        self.code = code
        super().__init__(message)


# ============================================================
# 主入口:execute
# ============================================================

def execute(action_id: str, target_resource_id: str,
            input_params: dict | None = None,
            initiated_by: str = "system",
            finding_id: str | None = None,
            request_reason: str = "",
            verify: bool = True) -> RecoveryExecution:
    """执行恢复动作。

    - low_risk + 不需要审批 → 同步执行,返回 status=succeeded/failed
    - medium / high_risk 或 requires_approval=True → 创建 awaiting_approval execution +
      ApprovalRequest,返回 status=awaiting_approval(调用方据此判断是否要审批)
    - verify=True(默认):succeeded 后自动跑 verifier;verify_failed → 自动 rollback

    抛 ExecutionError 表示**前置校验失败**(动作不存在、目标非法等);
    handler 内部失败不抛异常,而是 status=failed + result.error。
    """
    action = get_action(action_id)
    if action is None:
        raise ExecutionError(f"unknown action_id: {action_id}", 404)

    if not is_executable(action_id):
        raise ExecutionError(
            f"action '{action_id}' has no execute handler",
            code=501,
        )

    # 跑 dry_run(复用 cascade)
    dry_result = compute_dry_run(action_id, target_resource_id, input_params or {})
    if not dry_result["target_valid"]:
        raise ExecutionError(
            f"dry-run validation failed: {dry_result['validation_error']}",
            code=400,
        )

    # 创建 RecoveryExecution
    now_iso = datetime.now(timezone.utc).isoformat()
    target_node = store.get_node(target_resource_id)
    needs_approval = action["risk_level"] != "low" or action["requires_approval"]

    execution = RecoveryExecution(
        execution_id=str(uuid.uuid4()),
        action_id=action_id,
        target_resource_id=target_resource_id,
        target_resource_type=target_node.type if target_node else action["target_type"],
        finding_id=finding_id,
        input_params=dict(input_params or {}),
        dry_run_result=dry_result,
        status="awaiting_approval" if needs_approval else "executing",
        initiated_by=initiated_by,
        request_reason=request_reason,
        initiated_at=now_iso,
        executed_at="" if needs_approval else now_iso,
        cluster_id=_derive_cluster_id(target_resource_id, target_node),
    )
    store.add_execution(execution)

    if needs_approval:
        # 创建审批请求,返回(由 routers 给 202 响应)
        from app.recovery.approval import request_approval

        approval = request_approval(
            execution=execution,
            requested_by=initiated_by,
            request_reason=request_reason,
        )
        execution.approval_id = approval.approval_id
        # 把 verify 偏好记到 input_params 里,_continue_after_approval 复用
        if not verify:
            execution.input_params["__verify"] = False
        store.update_execution(execution)
        # awaiting_approval 不写 Neo4j(避免一个动作产生多次写入);由 _continue 阶段写
        return execution

    # low_risk → 同步执行
    return _run_handler_and_persist(execution, verify=verify)


# ============================================================
# Sprint 3:审批通过后继续执行
# ============================================================

def _continue_after_approval(execution_id: str) -> RecoveryExecution:
    """approve 端点调,标记 execution 进入 executing 并跑 handler。

    从 awaiting_approval 进入 executing → succeeded / failed。
    """
    execution = store.get_execution(execution_id)
    if execution is None:
        raise ExecutionError(f"execution not found: {execution_id}", 404)
    if execution.status != "awaiting_approval":
        raise ExecutionError(
            f"execution status is {execution.status}, expected awaiting_approval",
            code=409,
        )

    now_iso = datetime.now(timezone.utc).isoformat()
    execution.status = "executing"
    execution.executed_at = now_iso
    store.update_execution(execution)

    verify = execution.input_params.pop("__verify", True) if isinstance(execution.input_params, dict) else True
    return _run_handler_and_persist(execution, verify=verify)


# ============================================================
# Sprint 3:回滚
# ============================================================

def rollback(execution_id: str,
             initiated_by: str = "system",
             reason: str = "") -> RecoveryExecution:
    """对一个 succeeded execution 执行回滚。

    创建一个反向 execution(reverses_execution_id 指向原 exec),
    直接同步执行反向 handler,**不再二次审批**(用户已确认设计)。

    仅允许 status=succeeded 的 execution 回滚;rolled_back 后不可再回滚。
    """
    original = store.get_execution(execution_id)
    if original is None:
        raise ExecutionError(f"execution not found: {execution_id}", 404)
    if original.status != "succeeded":
        raise ExecutionError(
            f"only succeeded executions can be rolled back (current: {original.status})",
            code=409,
        )
    if original.rollback_execution_id:
        raise ExecutionError(
            f"execution already rolled back by {original.rollback_execution_id}",
            code=409,
        )

    action = get_action(original.action_id)
    rollback_action_id = (action or {}).get("rollback_action_id")
    if not rollback_action_id:
        raise ExecutionError(
            f"action '{original.action_id}' has no rollback_action_id",
            code=400,
        )

    return _do_rollback(original, initiated_by, reason, auto_rollback_marker=False)


def _do_rollback(original: RecoveryExecution,
                 initiated_by: str,
                 reason: str,
                 auto_rollback_marker: bool = False) -> RecoveryExecution:
    """实际创建并执行 rollback execution。

    供 `rollback()` 与 `_verify_and_maybe_rollback` 复用。
    auto_rollback_marker=True 时:
    - rollback execution 自身 **不再 verify**(预防 verify_failed → rollback → rollback verify_failed 死循环)
    - result 标 `auto_rollback_origin: <original_execution_id>`
    """
    action = get_action(original.action_id)
    rollback_action_id = (action or {}).get("rollback_action_id")
    if not rollback_action_id:
        raise ExecutionError(
            f"action '{original.action_id}' has no rollback_action_id",
            code=400,
        )

    # 反向参数:scale_deployment 的反向是 scale_deployment(replicas_delta 取反)
    rollback_params = _derive_rollback_params(original)

    # 跑 dry_run(允许失败但不阻塞 — 回滚是兜底操作)
    dry_result = compute_dry_run(rollback_action_id, original.target_resource_id, rollback_params)

    now_iso = datetime.now(timezone.utc).isoformat()
    rb_execution = RecoveryExecution(
        execution_id=str(uuid.uuid4()),
        action_id=rollback_action_id,
        target_resource_id=original.target_resource_id,
        target_resource_type=original.target_resource_type,
        finding_id=original.finding_id,
        input_params=rollback_params,
        dry_run_result=dry_result,
        status="executing",
        initiated_by=initiated_by,
        request_reason=reason or f"rollback of {original.execution_id}",
        initiated_at=now_iso,
        executed_at=now_iso,
        reverses_execution_id=original.execution_id,
        cluster_id=original.cluster_id,
    )
    store.add_execution(rb_execution)

    # 自动回滚:跳过 rollback 自身的 verify(防递归),且不要它再自动 rollback
    rb_execution = _run_handler_and_persist(
        rb_execution,
        auto_rollback=False,
        verify=not auto_rollback_marker,
    )
    if auto_rollback_marker:
        rb_execution.result["auto_rollback_origin"] = original.execution_id
        store.update_execution(rb_execution)

    # 若回滚成功,把原 execution 标 rolled_back;失败则原 execution 保持 succeeded
    if rb_execution.status == "succeeded":
        original.rollback_execution_id = rb_execution.execution_id
        original.status = "rolled_back"
        original.completed_at = datetime.now(timezone.utc).isoformat()
        store.update_execution(original)
        try:
            _persist_execution(original)
        except Exception as e:    # noqa
            original.result.setdefault("warnings", []).append(
                f"Neo4j rolled_back persist warning: {type(e).__name__}: {e}"
            )

    return rb_execution


def _derive_rollback_params(original: RecoveryExecution) -> dict:
    """根据原动作类型派生反向参数。

    scale_deployment: replicas_delta 取反
    其他动作:rollback_action_id 多为 None,Sprint 3 内只 scale 真正可回滚。
    其他场景直接复用原参数(handler 自行处理幂等)。
    """
    if original.action_id == "scale_deployment":
        delta = original.input_params.get("replicas_delta", 0)
        return {"replicas_delta": -delta}
    return dict(original.input_params)


def _derive_cluster_id(target_id: str, target_node) -> str:
    """从 target 派生 cluster_id。

    优先 target.properties["cluster_id"],次回 target_id 第二段(`<type>:<cluster>:...`),
    都缺 → 空串(MySQL/Redis 无集群概念,非 K8s 资源用空串)。
    """
    if target_node is not None:
        cid = (target_node.properties or {}).get("cluster_id")
        if cid:
            return cid
    parts = target_id.split(":")
    if len(parts) >= 2 and parts[1]:
        # 第二段通常是 cluster_id(K8s 资源);MySQL/Redis 可能也跟这个约定但非强制
        return parts[1]
    return ""


def reverify(execution_id: str) -> RecoveryExecution:
    """主动触发某 execution 的重新验证(不触发 auto rollback)。

    用于:① UI dashboard 刷新 verify_status ② succeeded 后等了一段时间想再确认状态。
    仅允许 status ∈ (succeeded, rolled_back);其他状态(awaiting_approval / failed)报 409。
    """
    execution = store.get_execution(execution_id)
    if execution is None:
        raise ExecutionError(f"execution not found: {execution_id}", 404)
    if execution.status not in ("succeeded", "rolled_back"):
        raise ExecutionError(
            f"reverify only allowed on succeeded/rolled_back executions (current: {execution.status})",
            code=409,
        )
    _verify_and_maybe_rollback(execution, auto_rollback=False)
    return execution


# ============================================================
# 列表 + 持久化
# ============================================================

def list_executions(status: str | None = None,
                    action_id: str | None = None,
                    target_resource_id: str | None = None,
                    limit: int = 50) -> list[RecoveryExecution]:
    """查询执行历史(从 DSS 内存)。

    新到旧排序。limit 默认 50,最大 500。
    """
    if limit > 500:
        limit = 500

    executions = store.get_all_executions()

    if status:
        executions = [e for e in executions if e.status == status]
    if action_id:
        executions = [e for e in executions if e.action_id == action_id]
    if target_resource_id:
        executions = [e for e in executions if e.target_resource_id == target_resource_id]

    executions.sort(key=lambda e: e.initiated_at or "", reverse=True)
    return executions[:limit]


def _run_handler_and_persist(execution: RecoveryExecution,
                              auto_rollback: bool = True,
                              verify: bool = True) -> RecoveryExecution:
    """执行 handler,更新 status,持久化到 Neo4j。供 execute / _continue / rollback 复用。

    auto_rollback=True(默认):succeeded 后若 verify_status=failed → 自动调 rollback。
    rollback 自身重入本函数时传 auto_rollback=False 防递归。
    verify=False:跳过 verifier(测试 / 用户显式关闭场景)。
    """
    handler = get_handler(execution.action_id)
    if handler is None:
        execution.result = {"success": False, "error": f"no handler for action {execution.action_id}"}
        execution.status = "failed"
        execution.completed_at = datetime.now(timezone.utc).isoformat()
        store.update_execution(execution)
        return execution

    context = {
        "execution_id": execution.execution_id,
        "initiated_by": execution.initiated_by,
        "auto_rollback": auto_rollback,
    }

    try:
        result = handler(execution.target_resource_id, execution.input_params, context)
    except Exception as e:    # noqa
        result = {"success": False, "error": f"handler raised: {type(e).__name__}: {e}"}

    execution.completed_at = datetime.now(timezone.utc).isoformat()
    execution.result = result
    execution.status = "succeeded" if result.get("success") else "failed"
    store.update_execution(execution)

    # Phase 2 余项 — 执行成功后跑 verifier
    if execution.status == "succeeded" and verify:
        _verify_and_maybe_rollback(execution, auto_rollback=auto_rollback)

    try:
        _persist_execution(execution)
    except Exception as e:    # noqa
        execution.result.setdefault("warnings", []).append(
            f"Neo4j persist warning: {type(e).__name__}: {e}"
        )

    return execution


def _verify_and_maybe_rollback(execution: RecoveryExecution, auto_rollback: bool) -> None:
    """跑 verifier;若 passed=False 且 auto_rollback=True → 触发自动回滚。

    verify_status 取值:passed | failed | skipped | not_supported | timeout | error
    """
    from app.recovery.verifiers import run_verifier

    context = {
        "execution_id": execution.execution_id,
        "initiated_by": execution.initiated_by,
    }
    verdict = run_verifier(
        execution.action_id,
        execution.target_resource_id,
        execution.input_params,
        execution.result,
        context,
    )
    execution.verify_result = verdict
    execution.verified_at = datetime.now(timezone.utc).isoformat()

    predicate = verdict.get("predicate", "")
    if predicate in ("skipped", "not_supported"):
        execution.verify_status = predicate
    elif verdict.get("passed"):
        execution.verify_status = "passed"
    elif predicate == "error":
        execution.verify_status = "error"
    else:
        execution.verify_status = "failed"

    store.update_execution(execution)

    # verify_failed + auto_rollback → 触发回滚(但仅当原动作有 rollback_action_id)
    if execution.verify_status == "failed" and auto_rollback:
        action = get_action(execution.action_id)
        rb_action_id = (action or {}).get("rollback_action_id")
        if not rb_action_id:
            execution.result.setdefault("warnings", []).append(
                f"verify_failed but action has no rollback_action_id, manual intervention needed"
            )
            store.update_execution(execution)
            return
        try:
            rb = _do_rollback(
                original=execution,
                initiated_by="auto-verifier",
                reason=f"auto rollback: verify_failed ({verdict.get('message', '')})",
                auto_rollback_marker=True,
            )
            execution.result["auto_rollback"] = {
                "triggered": True,
                "rollback_execution_id": rb.execution_id,
                "rollback_status": rb.status,
            }
            store.update_execution(execution)
        except Exception as e:  # noqa: BLE001
            execution.result.setdefault("warnings", []).append(
                f"auto rollback failed: {type(e).__name__}: {e}"
            )
            store.update_execution(execution)


def _persist_execution(execution: RecoveryExecution):
    """写入 Neo4j 作为 RecoveryExecution 节点。"""
    driver = get_driver()
    if driver is None:
        return

    with driver.session() as s:
        s.run("""
            MERGE (e:RecoveryExecution:ResourceInstance {node_id: $eid})
            SET e.execution_id = $eid,
                e.action_id = $aid,
                e.target_resource_id = $tid,
                e.target_resource_type = $ttype,
                e.finding_id = $fid,
                e.status = $status,
                e.initiated_by = $by,
                e.initiated_at = $iat,
                e.executed_at = $eat,
                e.completed_at = $cat,
                e.request_reason = $reason,
                e.result_json = $rjson,
                e.reverses_execution_id = $reverses,
                e.cluster_id = $cluster,
                e.verify_status = $vstatus,
                e.verified_at = $vat,
                e.label = 'RecoveryExecution',
                e.name = $name,
                e.health_status = $health,
                e.version = 'v1',
                e.updated_at = datetime()
        """,
              eid=execution.execution_id,
              aid=execution.action_id,
              tid=execution.target_resource_id,
              ttype=execution.target_resource_type,
              fid=execution.finding_id or "",
              status=execution.status,
              by=execution.initiated_by,
              iat=execution.initiated_at,
              eat=execution.executed_at,
              cat=execution.completed_at,
              reason=execution.request_reason,
              rjson=str(execution.result),
              reverses=execution.reverses_execution_id or "",
              cluster=execution.cluster_id or "",
              vstatus=execution.verify_status or "",
              vat=execution.verified_at or "",
              name=f"{execution.action_id} on {execution.target_resource_id}",
              health="normal" if execution.status == "succeeded" else "critical",
              )

        # 关联到 target resource (TARGETS)
        s.run("""
            MATCH (e:RecoveryExecution {execution_id: $eid})
            MATCH (t:ResourceInstance {node_id: $tid})
            MERGE (e)-[r:RELATES_TO {edge_id: 'exec_target_' + $eid}]->(t)
            SET r.relationship_type = 'TARGETS',
                r.relationship_name = '针对',
                r.dependency_strength = '强',
                r.last_verified_at = datetime(),
                r.version = 'v1'
        """, eid=execution.execution_id, tid=execution.target_resource_id)

        # 如果 finding_id 非空,关联 TRIGGERED_BY
        if execution.finding_id:
            s.run("""
                MATCH (e:RecoveryExecution {execution_id: $eid})
                MATCH (f:InspectionFinding {node_id: $fid})
                MERGE (e)-[r:RELATES_TO {edge_id: 'exec_trig_' + $eid}]->(f)
                SET r.relationship_type = 'TRIGGERED_BY',
                    r.relationship_name = '触发自',
                    r.dependency_strength = '中',
                    r.last_verified_at = datetime(),
                    r.version = 'v1'
            """, eid=execution.execution_id, fid=execution.finding_id)

        # 回滚 execution 关联到原 execution(REVERSES)
        if execution.reverses_execution_id:
            s.run("""
                MATCH (e:RecoveryExecution {execution_id: $eid})
                MATCH (orig:RecoveryExecution {execution_id: $oid})
                MERGE (e)-[r:RELATES_TO {edge_id: 'exec_rev_' + $eid}]->(orig)
                SET r.relationship_type = 'REVERSES',
                    r.relationship_name = '回滚',
                    r.dependency_strength = '强',
                    r.last_verified_at = datetime(),
                    r.version = 'v1'
            """, eid=execution.execution_id, oid=execution.reverses_execution_id)
