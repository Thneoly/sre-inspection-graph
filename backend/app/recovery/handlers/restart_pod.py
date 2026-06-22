"""restart_pod 执行器 — Phase 2 真实 K8s + mock 双模式。

real:`CoreV1Api.delete_namespaced_pod` 删 Pod(控制器会拉起新的)。成功后更新 DSS 孪生。
mock(默认):仅改 DSS restart_count。
"""

from datetime import datetime, timezone

from app.config import settings
from app.datasource.connectors.k8s_client import get_k8s_core_api, k8s_ref, run_k8s
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

    old_restart_count = int(target.properties.get("restart_count", 0))
    new_restart_count = old_restart_count + 1

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, target, old_restart_count, new_restart_count,
                             graceful, grace_period, context)
    return _execute_mock(target_id, target, old_restart_count, new_restart_count,
                         graceful, grace_period, context)


def _apply_dss(target_id, target, new_restart_count, context):
    now = datetime.now(timezone.utc).isoformat()
    new_props = {
        "restart_count": new_restart_count,
        "last_restarted_at": now,
        "last_restarted_by_execution": context.get("execution_id", ""),
    }
    if target.properties.get("health_status") == "warning":
        new_props["health_status"] = "normal"
    store.update_node_props(target_id, **new_props)
    return now


def _execute_mock(target_id, target, old, new, graceful, grace_period, context) -> dict:
    now = _apply_dss(target_id, target, new, context)
    return {
        "success": True,
        "completed_at": now,
        "old_restart_count": old,
        "new_restart_count": new,
        "graceful": graceful,
        "grace_period_seconds": grace_period,
        "note": f"Pod {target.name} restarted (mock execution, count={new})",
    }


def _execute_real(target_id, target, old, new, graceful, grace_period, context) -> dict:
    try:
        namespace, name = k8s_ref(target_id)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    async def _call():
        from kubernetes_asyncio.client import V1DeleteOptions
        api, core = await get_k8s_core_api()
        try:
            opts = V1DeleteOptions(grace_period_seconds=grace_period)
            await core.delete_namespaced_pod(name=name, namespace=namespace, body=opts)
        finally:
            await api.close()

    try:
        run_k8s(_call())
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"k8s delete pod failed: {type(e).__name__}: {e}"}

    now = _apply_dss(target_id, target, new, context)
    return {
        "success": True,
        "completed_at": now,
        "old_restart_count": old,
        "new_restart_count": new,
        "graceful": graceful,
        "grace_period_seconds": grace_period,
        "namespace": namespace,
        "name": name,
        "note": f"Pod {target.name} restarted (real k8s execution, count={new})",
    }
