"""restart_service 执行器 — Sprint 2 mock 实现。

真实环境会调:
    kubectl delete endpoints <svc-name>  (强制重新生成)
    或通过 client-go: CoreV1Api.delete_namespaced_endpoints()

Sprint 2 mock:更新 Service 节点的 properties.endpoints_refreshed_at 时间戳,
代表 Endpoints 已重新生成。
"""

from datetime import datetime, timezone
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """重启 Service Endpoints。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Service":
        return {"success": False, "error": f"target is {target.type}, not Service"}

    now = datetime.now(timezone.utc).isoformat()
    refresh_count = int(target.properties.get("endpoints_refresh_count", 0)) + 1

    store.update_node_props(target_id,
                            endpoints_refreshed_at=now,
                            endpoints_refresh_count=refresh_count,
                            last_refreshed_by_execution=context.get("execution_id", ""))

    return {
        "success": True,
        "completed_at": now,
        "endpoints_refresh_count": refresh_count,
        "note": f"Service {target.name} endpoints refreshed (mock execution, count={refresh_count})",
    }
