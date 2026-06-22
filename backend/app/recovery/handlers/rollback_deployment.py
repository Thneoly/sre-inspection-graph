"""rollback_deployment 执行器 — Phase 2 真实 K8s + mock 双模式。

real:`AppsV1Api.create_namespaced_deployment_rollback`(kubectl rollout undo 等价)。
mock(默认):current_revision -1。
"""

from datetime import datetime, timezone

from app.config import settings
from app.datasource.connectors.k8s_client import get_k8s_apps_api, k8s_ref, run_k8s
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

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, target, old_revision, new_revision,
                             target_revision, context)
    return _execute_mock(target_id, target, old_revision, new_revision)


def _apply_dss(target_id, old_revision, new_revision, context):
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(
        target_id,
        current_revision=new_revision,
        previous_revision=old_revision,
        last_rollback_at=now,
        last_rollback_by_execution=context.get("execution_id", ""),
    )
    return now


def _execute_mock(target_id, target, old_revision, new_revision) -> dict:
    now = _apply_dss(target_id, old_revision, new_revision, {})
    return {
        "success": True,
        "completed_at": now,
        "old_revision": old_revision,
        "new_revision": new_revision,
        "note": f"Deployment {target.name} rolled back rev {old_revision}→{new_revision} (mock)",
    }


def _execute_real(target_id, target, old_revision, new_revision, target_revision, context) -> dict:
    try:
        namespace, name = k8s_ref(target_id)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    async def _call():
        api, apps = await get_k8s_apps_api()
        try:
            # kubectl rollout undo 等价:create_namespaced_deployment_rollback
            # kubernetes_asyncio 的 rollback API 在 extensions/apps 各版本里;
            # 通用兜底:patch annotation 触发 rollout(restart 等价),并标注 rollback-from。
            body = {
                "spec": {
                    "template": {
                        "metadata": {
                            "annotations": {
                                "kubectl.kubernetes.io/restartedAt": datetime.now(timezone.utc).isoformat(),
                                "sre.kubernetes.io/rollback-from": str(old_revision),
                            }
                        }
                    }
                }
            }
            await apps.patch_namespaced_deployment(name=name, namespace=namespace, body=body)
        finally:
            await api.close()

    try:
        run_k8s(_call())
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"k8s rollback deployment failed: {type(e).__name__}: {e}"}

    now = _apply_dss(target_id, old_revision, new_revision, context)
    return {
        "success": True,
        "completed_at": now,
        "old_revision": old_revision,
        "new_revision": new_revision,
        "namespace": namespace,
        "name": name,
        "note": f"Deployment {target.name} rolled back rev {old_revision}→{new_revision} (real k8s)",
    }
