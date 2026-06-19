"""refresh_secret 执行器 — Sprint 3 mock 实现。

真实环境会调:
    kubectl create secret generic <name> --from-literal=...
    然后 kubectl rollout restart deployment 触发引用 Pod 重启

Sprint 3 mock:递增 secret_version,更新 refreshed_at;
若 trigger_pod_restart=True,标记所有 USES 反向引用的 Pod 为待重启。
"""

from datetime import datetime, timezone
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """刷新 Secret。"""
    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Secret":
        return {"success": False, "error": f"target is {target.type}, not Secret"}

    trigger_pod_restart = params.get("trigger_pod_restart", True)

    now = datetime.now(timezone.utc).isoformat()
    old_version = int(target.properties.get("secret_version", 1))
    new_version = old_version + 1

    store.update_node_props(
        target_id,
        secret_version=new_version,
        refreshed_at=now,
        refreshed_by_execution=context.get("execution_id", ""),
    )

    affected_pods: list[str] = []
    if trigger_pod_restart:
        # 找所有 USES → target_id 的 Pod(反向)
        for edge in store.get_all_edges():
            if edge.relationship_type != "USES":
                continue
            if edge.target_id != target_id:
                continue
            pod = store.get_node(edge.source_id)
            if pod and pod.type == "Pod":
                affected_pods.append(pod.id)
                store.update_node_props(
                    pod.id,
                    pending_restart=True,
                    pending_restart_reason=f"secret refresh ({target_id})",
                    pending_restart_at=now,
                )

    return {
        "success": True,
        "completed_at": now,
        "old_version": old_version,
        "new_version": new_version,
        "trigger_pod_restart": trigger_pod_restart,
        "affected_pod_count": len(affected_pods),
        "affected_pods": affected_pods,
        "note": f"Secret {target.name} refreshed v{old_version}→v{new_version} (mock)"
                + (f", {len(affected_pods)} pod(s) marked for restart" if affected_pods else ""),
    }
