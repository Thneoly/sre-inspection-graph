"""refresh_secret 执行器 — Phase 2 真实 K8s + mock 双模式。

real:`CoreV1Api.patch_namespaced_secret` 重写 data(resourceVersion 自增)。
**不自动 rollout restart**(降级 —— 影响面大,由运维手动触发),仅标记 USES 反向 Pod `pending_restart`。
mock(默认):同 mock 逻辑递增 secret_version + 标记 Pod。
"""

from datetime import datetime, timezone

from app.config import settings
from app.datasource.connectors.k8s_client import get_k8s_core_api, k8s_ref, run_k8s
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """刷新 Secret。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Secret":
        return {"success": False, "error": f"target is {target.type}, not Secret"}

    trigger_pod_restart = params.get("trigger_pod_restart", True)
    old_version = int(target.properties.get("secret_version", 1))
    new_version = old_version + 1

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, target, old_version, new_version,
                             trigger_pod_restart, context)
    return _execute_mock(target_id, target, old_version, new_version,
                         trigger_pod_restart, context)


def _mark_affected_pods(target_id, now):
    """标记所有 USES → target_id 的 Pod 为 pending_restart。返回 pod id 列表。"""
    affected: list[str] = []
    for edge in store.get_all_edges():
        if edge.relationship_type != "USES":
            continue
        if edge.target_id != target_id:
            continue
        pod = store.get_node(edge.source_id)
        if pod and pod.type == "Pod":
            affected.append(pod.id)
            store.update_node_props(
                pod.id,
                pending_restart=True,
                pending_restart_reason=f"secret refresh ({target_id})",
                pending_restart_at=now,
            )
    return affected


def _execute_mock(target_id, target, old_version, new_version, trigger_pod_restart, context) -> dict:
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(
        target_id,
        secret_version=new_version,
        refreshed_at=now,
        refreshed_by_execution=context.get("execution_id", ""),
    )
    affected = _mark_affected_pods(target_id, now) if trigger_pod_restart else []
    return _result(target, old_version, new_version, trigger_pod_restart, affected, now, "mock")


def _execute_real(target_id, target, old_version, new_version, trigger_pod_restart, context) -> dict:
    try:
        namespace, name = k8s_ref(target_id)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    # 新 data:无显式 new_data 入参 → 用 existing data 原样回写(触发 resourceVersion 自增,代表"已轮转")
    new_data = params_data(target)

    async def _call():
        api, core = await get_k8s_core_api()
        try:
            # patch secret 的 data(空 dict 也让 resourceVersion 自增,代表"已轮转")
            await core.patch_namespaced_secret(name=name, namespace=namespace, body={"data": new_data})
        finally:
            await api.close()

    try:
        run_k8s(_call())
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"k8s patch secret failed: {type(e).__name__}: {e}"}

    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(
        target_id,
        secret_version=new_version,
        refreshed_at=now,
        refreshed_by_execution=context.get("execution_id", ""),
    )
    affected = _mark_affected_pods(target_id, now) if trigger_pod_restart else []
    return _result(target, old_version, new_version, trigger_pod_restart, affected, now, "real k8s",
                   namespace=namespace, name=name)


def params_data(target):
    """real 模式下构造新 data。无显式 new_data 入参 → 用 existing data 原样回写(触发轮转标记)。"""
    existing = (target.properties or {}).get("data") or {}
    return dict(existing)


def _result(target, old_version, new_version, trigger_pod_restart, affected, now, mode,
            namespace=None, name=None) -> dict:
    out = {
        "success": True,
        "completed_at": now,
        "old_version": old_version,
        "new_version": new_version,
        "trigger_pod_restart": trigger_pod_restart,
        "affected_pod_count": len(affected),
        "affected_pods": affected,
        "note": f"Secret {target.name} refreshed v{old_version}→v{new_version} ({mode})"
                + (f", {len(affected)} pod(s) marked for restart" if affected else ""),
    }
    if namespace:
        out["namespace"] = namespace
        out["name"] = name
    return out
