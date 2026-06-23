"""scale_deployment 执行器 — Phase 2 真实 K8s + mock 双模式。

real 模式:`AppsV1Api.patch_namespaced_deployment_scale` 调真实集群,成功后更新 DSS 孪生。
mock 模式(默认):仅改 DSS properties,测试安全。
"""

from datetime import datetime, timezone

from app.config import settings
from app.datasource.connectors.k8s_client import get_k8s_apps_api, k8s_ref, run_k8s
from app.datasource.store import store


def execute(target_id: str, params: dict, context: dict) -> dict:
    """执行扩缩容。

    Args:
        target_id: deploy:vm-cluster:otel-demo:cart
        params: {"replicas_delta": 2}
        context: {"execution_id": "...", "initiated_by": "..."}
    """
    delta = params.get("replicas_delta", 0)
    if delta == 0:
        return {"success": False, "error": "replicas_delta must be non-zero"}

    target = store.get_node(target_id)
    if not target:
        return {"success": False, "error": f"target not found: {target_id}"}
    if target.type != "Deployment":
        return {"success": False, "error": f"target is {target.type}, not Deployment"}

    old_replicas = int(target.properties.get("desired_replicas", 3))
    new_replicas = old_replicas + delta

    if new_replicas < 0:
        return {"success": False, "error": f"new replicas would be negative ({new_replicas})"}
    if new_replicas > 100:
        return {"success": False, "error": f"new replicas exceeds limit ({new_replicas} > 100)"}

    if settings.recovery_handler_mode == "real":
        return _execute_real(target_id, old_replicas, new_replicas, delta, context)
    return _execute_mock(target_id, old_replicas, new_replicas, delta)


def _execute_mock(target_id, old, new, delta) -> dict:
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(target_id,
                            desired_replicas=new,
                            available_replicas=new,
                            scaled_at=now,
                            scaled_by_execution="")
    return {
        "success": True,
        "old_replicas": old,
        "new_replicas": new,
        "delta_applied": delta,
        "completed_at": now,
        "note": f"Deployment scaled from {old} to {new} replicas (mock execution)",
    }


def _execute_real(target_id, old, new, delta, context) -> dict:
    """先调 K8s API,成功后再更新 DSS 孪生。失败不动 DSS。"""
    try:
        cluster_id, namespace, name = k8s_ref(target_id)
    except ValueError as e:
        return {"success": False, "error": str(e)}

    async def _call():
        api, apps = await get_k8s_apps_api(cluster_id)
        try:
            # patch_namespaced_deployment_scale:body 用 V1Scale { spec: { replicas: new } }
            from kubernetes_asyncio.client import V1Scale, V1ScaleSpec
            body = V1Scale(spec=V1ScaleSpec(replicas=new))
            await apps.patch_namespaced_deployment_scale(
                name=name, namespace=namespace, body=body,
            )
        finally:
            await api.close()

    try:
        run_k8s(_call())
    except Exception as e:  # noqa: BLE001
        return {"success": False, "error": f"k8s patch scale failed: {type(e).__name__}: {e}",
                "old_replicas": old, "new_replicas": new}

    # API 成功 → 更新 DSS 孪生
    now = datetime.now(timezone.utc).isoformat()
    store.update_node_props(target_id,
                            desired_replicas=new,
                            available_replicas=new,
                            scaled_at=now,
                            scaled_by_execution=context.get("execution_id", ""))
    return {
        "success": True,
        "old_replicas": old,
        "new_replicas": new,
        "delta_applied": delta,
        "completed_at": now,
        "cluster_id": cluster_id,
        "namespace": namespace,
        "name": name,
        "note": f"Deployment scaled from {old} to {new} replicas (real k8s execution, cluster={cluster_id})",
    }
