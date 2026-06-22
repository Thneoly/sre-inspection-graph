"""drain_node 执行器 — Phase 2 真实 K8s + mock 双模式。

real:`CoreV1Api.patch_node` 设 `unschedulable=True`(cordon)。**不真删 Pod**(降级 ——
真实 evict 误删生产风险高,且需 PodDisruptionBudget 协调),仅标记 DSS Pod `eviction_pending`。
运维确认后手动 `kubectl drain` 完成。CLAUDE 标注真实 evict 留 Phase 3。
mock(默认):同 mock 逻辑。
"""

from datetime import datetime, timezone

from app.config import settings
from app.datasource.connectors.k8s_client import get_k8s_core_api, k8s_ref, run_k8s
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """驱逐 Node 上的 Pod。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "KubernetesNode":
        return {"success": False, "error": f"target is {target.type}, not KubernetesNode"}

    ignore_daemonsets = params.get("ignore_daemonsets", True)
    delete_local_data = params.get("delete_local_data", False)
    force = params.get("force", False)

    pods_on_node: list[str] = []
    for edge in store.get_all_edges():
        if edge.relationship_type != "SCHEDULED_ON":
            continue
        if edge.target_id != target_id:
            continue
        pod = store.get_node(edge.source_id)
        if pod and pod.type == "Pod":
            pods_on_node.append(pod.id)

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, target, pods_on_node,
                             ignore_daemonsets, delete_local_data, force, context)
    return _execute_mock(target_id, target, pods_on_node,
                         ignore_daemonsets, delete_local_data, force, context)


def _apply_dss(target_id, pods_on_node, context):
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(
        target_id,
        cordoned=True,
        drained_at=now,
        drained_by_execution=context.get("execution_id", ""),
        drain_pod_count=len(pods_on_node),
    )
    for pid in pods_on_node:
        store.update_node_props(
            pid,
            eviction_pending=True,
            eviction_reason=f"node drained ({target_id})",
            eviction_at=now,
        )
    return now


def _execute_mock(target_id, target, pods_on_node,
                  ignore_daemonsets, delete_local_data, force, context) -> dict:
    now = _apply_dss(target_id, pods_on_node, context)
    return _result(target, pods_on_node, ignore_daemonsets, delete_local_data, force, now, "mock")


def _execute_real(target_id, target, pods_on_node,
                  ignore_daemonsets, delete_local_data, force, context) -> dict:
    try:
        # Node 的 (namespace, name) — Node 是集群级资源,namespace 为空,name 从 properties 读
        namespace, name = k8s_ref(target_id)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    async def _call():
        api, core = await get_k8s_core_api()
        try:
            # cordon:patch node spec.unschedulable=True
            await core.patch_node(name=name, body={"spec": {"unschedulable": True}})
            # 不真删 Pod(降级)—— 仅 cordon。evict 留 Phase 3 + PDB 协调。
        finally:
            await api.close()

    try:
        run_k8s(_call())
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"k8s cordon node failed: {type(e).__name__}: {e}"}

    now = _apply_dss(target_id, pods_on_node, context)
    return _result(target, pods_on_node, ignore_daemonsets, delete_local_data, force, now,
                   "real k8s (cordon only, evict deferred)", name=name)


def _result(target, pods_on_node, ignore_daemonsets, delete_local_data, force, now, mode,
            name=None) -> dict:
    out = {
        "success": True,
        "completed_at": now,
        "cordoned": True,
        "ignore_daemonsets": ignore_daemonsets,
        "delete_local_data": delete_local_data,
        "force": force,
        "drained_pod_count": len(pods_on_node),
        "drained_pods": pods_on_node,
        "note": f"Node {target.name} cordoned, {len(pods_on_node)} pod(s) marked for eviction ({mode})",
    }
    if name:
        out["name"] = name
    return out
