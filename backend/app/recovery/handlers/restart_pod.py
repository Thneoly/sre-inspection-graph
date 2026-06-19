"""restart_pod 执行器 — Sprint 3 mock 实现。

真实环境会调:
    kubectl delete pod <name>
    或 client-go: CoreV1Api.delete_namespaced_pod()

Sprint 3 mock:增加 Pod 节点的 restart_count,刷新 last_restarted_at,
若 health_status=warning 复位为 normal(代表重启缓解了短期问题)。
"""

from datetime import datetime, timezone
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """重启 Pod。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Pod":
        return {"success": False, "error": f"target is {target.type}, not Pod"}

    graceful = params.get("graceful", True)
    grace_period = int(params.get("grace_period_seconds", 30))
    if grace_period < 0 or grace_period > 300:
        return {"success": False, "error": f"grace_period_seconds out of range: {grace_period}"}

    now = datetime.now(timezone.utc).isoformat()
    old_restart_count = int(target.properties.get("restart_count", 0))
    new_restart_count = old_restart_count + 1

    new_props = {
        "restart_count": new_restart_count,
        "last_restarted_at": now,
        "last_restarted_by_execution": context.get("execution_id", ""),
    }
    # warning → normal(重启缓解短期问题);critical 不动
    if target.properties.get("health_status") == "warning":
        new_props["health_status"] = "normal"

    store.update_node_props(target_id, **new_props)

    return {
        "success": True,
        "completed_at": now,
        "old_restart_count": old_restart_count,
        "new_restart_count": new_restart_count,
        "graceful": graceful,
        "grace_period_seconds": grace_period,
        "note": f"Pod {target.name} restarted (mock execution, count={new_restart_count})",
    }
