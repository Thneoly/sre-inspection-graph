"""drain_node 执行器 — Sprint 3 mock 实现。

真实环境会调:
    kubectl cordon <node> && kubectl drain <node> --ignore-daemonsets
    或 client-go: CoreV1Api.patch_node() 设 unschedulable + delete pods

Sprint 3 mock:标记 KubernetesNode.cordoned=True,记录 drained_at,
统计 SCHEDULED_ON 反向边上的 Pod 数。
"""

from datetime import datetime, timezone
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

    # 数节点上的 Pod
    pods_on_node: list[str] = []
    for edge in store.get_all_edges():
        if edge.relationship_type != "SCHEDULED_ON":
            continue
        if edge.target_id != target_id:
            continue
        pod = store.get_node(edge.source_id)
        if pod and pod.type == "Pod":
            pods_on_node.append(pod.id)

    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(
        target_id,
        cordoned=True,
        drained_at=now,
        drained_by_execution=context.get("execution_id", ""),
        drain_pod_count=len(pods_on_node),
    )

    # 把 Pod 标记为 evicted
    for pid in pods_on_node:
        store.update_node_props(
            pid,
            eviction_pending=True,
            eviction_reason=f"node drained ({target_id})",
            eviction_at=now,
        )

    return {
        "success": True,
        "completed_at": now,
        "cordoned": True,
        "ignore_daemonsets": ignore_daemonsets,
        "delete_local_data": delete_local_data,
        "force": force,
        "drained_pod_count": len(pods_on_node),
        "drained_pods": pods_on_node,
        "note": f"Node {target.name} cordoned + drained, {len(pods_on_node)} pod(s) marked for eviction (mock)",
    }
