"""RecoveryExecution Lifecycle 编排 — PRD-001 Sprint 2。

执行流程:
    pending  → 创建 RecoveryExecution
       ↓
    dry_run_ok → 跑 cascade.dry_run 验证目标合法
       ↓
    [low_risk]                    [medium/high_risk - Sprint 3]
       ↓                                   ↓
    executing                      awaiting_approval
       ↓                                   ↓
    succeeded / failed             approved → executing → succeeded / failed

Sprint 2 范围:
- 只支持 low_risk 动作(scale_deployment / kill_query / restart_service)
- 同步执行(handler 是 mock,瞬时返回)
- 持久化到 DSS + Neo4j
- medium/high_risk 返 501 Not Implemented(Sprint 3 加审批流)
"""

import uuid
from datetime import datetime, timezone

from app.datasource.models import RecoveryExecution
from app.datasource.store import store
from app.db.neo4j_client import get_driver
from app.recovery.action_defs import ACTION_DEFS, get_action
from app.recovery.cascade import dry_run as compute_dry_run
from app.recovery.handlers import get_handler, is_executable


class ExecutionError(Exception):
    """执行流程异常。message 直接给用户看。"""
    def __init__(self, message: str, code: int = 400):
        self.message = message
        self.code = code
        super().__init__(message)


def execute(action_id: str, target_resource_id: str,
            input_params: dict | None = None,
            initiated_by: str = "system",
            finding_id: str | None = None,
            request_reason: str = "") -> RecoveryExecution:
    """执行恢复动作完整流程。

    流程:
      1. 验证 action 存在
      2. Sprint 2 限制:只允许 low_risk + 已实现 handler 的动作
      3. 跑 dry_run 验证目标合法
      4. 创建 RecoveryExecution(status=executing)
      5. 调用 handler
      6. 更新 status = succeeded / failed
      7. 持久化到 Neo4j
      8. 返回 execution

    抛 ExecutionError 表示**前置校验失败**(动作不存在、目标非法、需要审批等);
    handler 内部失败不抛异常,而是 status=failed + result.error。
    """
    action = get_action(action_id)
    if action is None:
        raise ExecutionError(f"unknown action_id: {action_id}", 404)

    # Sprint 2 限制
    if action["risk_level"] != "low":
        raise ExecutionError(
            f"action '{action_id}' is {action['risk_level']} risk, "
            f"approval flow not implemented yet (Sprint 3)",
            code=501,
        )
    if action["requires_approval"]:
        raise ExecutionError(
            f"action '{action_id}' requires approval, not implemented yet (Sprint 3)",
            code=501,
        )
    if not is_executable(action_id):
        raise ExecutionError(
            f"action '{action_id}' has no execute handler in Sprint 2",
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
    execution = RecoveryExecution(
        execution_id=str(uuid.uuid4()),
        action_id=action_id,
        target_resource_id=target_resource_id,
        target_resource_type=target_node.type if target_node else action["target_type"],
        finding_id=finding_id,
        input_params=dict(input_params or {}),
        dry_run_result=dry_result,
        status="executing",
        initiated_by=initiated_by,
        request_reason=request_reason,
        initiated_at=now_iso,
        executed_at=now_iso,
    )
    store.add_execution(execution)

    # 调用 handler
    handler = get_handler(action_id)
    context = {
        "execution_id": execution.execution_id,
        "initiated_by": initiated_by,
    }

    try:
        result = handler(target_resource_id, input_params or {}, context)
    except Exception as e:    # noqa — handler 不应抛但兜底
        result = {"success": False, "error": f"handler raised: {type(e).__name__}: {e}"}

    # 更新状态
    execution.completed_at = datetime.now(timezone.utc).isoformat()
    execution.result = result
    execution.status = "succeeded" if result.get("success") else "failed"
    store.update_execution(execution)

    # 持久化到 Neo4j(失败不影响内存执行结果)
    try:
        _persist_execution(execution)
    except Exception as e:    # noqa
        execution.result.setdefault("warnings", []).append(
            f"Neo4j persist warning: {type(e).__name__}: {e}"
        )

    return execution


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

    # 按 initiated_at 倒序
    executions.sort(key=lambda e: e.initiated_at or "", reverse=True)
    return executions[:limit]


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
