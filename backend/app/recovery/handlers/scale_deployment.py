"""scale_deployment 执行器 — Sprint 2 mock 实现。

真实环境会调:
    kubectl scale deployment <name> --replicas=<n>
    或 client-go: AppsV1Api.patch_namespaced_deployment_scale()

Sprint 2 mock:在 DSS deploy 节点的 properties.replicas / available_replicas
字段上加 delta,代表扩缩容生效。
"""

from datetime import datetime, timezone
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """执行扩缩容。

    Args:
        target_id: deploy:cce-prod-01:order:order-api
        params: {"replicas_delta": 2}
        context: {"execution_id": "...", "initiated_by": "..."}

    Returns:
        result dict — 写入 RecoveryExecution.result
    """
    delta = params.get("replicas_delta", 0)
    if delta == 0:
        return {"success": False, "error": "replicas_delta must be non-zero"}

    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Deployment":
        return {"success": False, "error": f"target is {target.type}, not Deployment"}

    # 读当前副本数(默认 3)
    old_replicas = int(target.properties.get("desired_replicas", 3))
    new_replicas = old_replicas + delta

    if new_replicas < 0:
        return {"success": False, "error": f"new replicas would be negative ({new_replicas})"}
    if new_replicas > 100:
        return {"success": False, "error": f"new replicas exceeds limit ({new_replicas} > 100)"}

    # 更新 DSS 状态
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(target_id,
                            desired_replicas=new_replicas,
                            available_replicas=new_replicas,
                            scaled_at=now,
                            scaled_by_execution=context.get("execution_id", ""))

    return {
        "success": True,
        "old_replicas": old_replicas,
        "new_replicas": new_replicas,
        "delta_applied": delta,
        "completed_at": now,
        "note": f"Deployment scaled from {old_replicas} to {new_replicas} replicas (mock execution)",
    }
