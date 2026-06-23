"""restart_service 执行器 — Phase 2 真实 K8s + mock 双模式。

real:`CoreV1Api.delete_namespaced_endpoints` 删 Endpoints(kube-controller-manager 会重建)。
mock(默认):仅改 DSS endpoints_refresh_count。
"""

from datetime import datetime, timezone

from app.config import settings
from app.datasource.connectors.k8s_client import get_k8s_core_api, k8s_ref, run_k8s
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """重启 Service Endpoints。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Service":
        return {"success": False, "error": f"target is {target.type}, not Service"}

    refresh_count = int(target.properties.get("endpoints_refresh_count", 0)) + 1

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, target, refresh_count, context)
    return _execute_mock(target_id, target, refresh_count, context)


def _apply_dss(target_id, refresh_count, context):
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(target_id,
                            endpoints_refreshed_at=now,
                            endpoints_refresh_count=refresh_count,
                            last_refreshed_by_execution=context.get("execution_id", ""))
    return now


def _execute_mock(target_id, target, refresh_count, context) -> dict:
    now = _apply_dss(target_id, refresh_count, context)
    return {
        "success": True,
        "completed_at": now,
        "endpoints_refresh_count": refresh_count,
        "note": f"Service {target.name} endpoints refreshed (mock execution, count={refresh_count})",
    }


def _execute_real(target_id, target, refresh_count, context) -> dict:
    try:
        cluster_id, namespace, name = k8s_ref(target_id)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    async def _call():
        api, core = await get_k8s_core_api(cluster_id)
        try:
            # 删 Endpoints,kube-controller-manager 会重建(同 kubectl delete endpoints)
            await core.delete_namespaced_endpoints(name=name, namespace=namespace)
        finally:
            await api.close()

    try:
        run_k8s(_call())
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"k8s delete endpoints failed: {type(e).__name__}: {e}"}

    now = _apply_dss(target_id, refresh_count, context)
    return {
        "success": True,
        "completed_at": now,
        "endpoints_refresh_count": refresh_count,
        "cluster_id": cluster_id,
        "namespace": namespace,
        "name": name,
        "note": f"Service {target.name} endpoints refreshed (real k8s execution, cluster={cluster_id}, count={refresh_count})",
    }
