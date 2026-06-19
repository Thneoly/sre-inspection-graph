"""rollback_deployment 执行器 — Sprint 3 mock 实现。

真实环境会调:
    kubectl rollout undo deployment <name>
    或 client-go: AppsV1Api.create_namespaced_deployment_rollback()

Sprint 3 mock:把 Deployment.current_revision 减 1(默认 2 → 1),
记录 last_rollback_at。如果指定 revision,直接设置。
"""

from datetime import datetime, timezone
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """回滚 Deployment 版本。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Deployment":
        return {"success": False, "error": f"target is {target.type}, not Deployment"}

    old_revision = int(target.properties.get("current_revision", 2))
    target_revision = params.get("revision")

    if target_revision is not None:
        if not isinstance(target_revision, int) or target_revision < 1:
            return {"success": False, "error": f"invalid revision: {target_revision}"}
        if target_revision >= old_revision:
            return {
                "success": False,
                "error": f"target revision {target_revision} not older than current {old_revision}",
            }
        new_revision = target_revision
    else:
        new_revision = old_revision - 1

    if new_revision < 1:
        return {"success": False, "error": f"cannot rollback below revision 1 (current={old_revision})"}

    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(
        target_id,
        current_revision=new_revision,
        previous_revision=old_revision,
        last_rollback_at=now,
        last_rollback_by_execution=context.get("execution_id", ""),
    )

    return {
        "success": True,
        "completed_at": now,
        "old_revision": old_revision,
        "new_revision": new_revision,
        "note": f"Deployment {target.name} rolled back from rev {old_revision} → {new_revision} (mock)",
    }
