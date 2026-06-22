"""K8s 客户端工厂 — PRD-001 Phase 2 recovery handler 专用。

供 recovery handler 真实模式调 K8s API。**不复用给 connector** —— connector 是
多集群(每实例一个 cluster,实例级 `_k8s_loaded`),本工厂是单集群(模块级
`_loaded_cluster` = `settings.active_cluster`),语义不同。多集群 handler dispatch
留 Phase 3。

`kubernetes_asyncio` 的 config 加载是**全局状态**(load_kube_config 写入模块级
configuration),故本模块用 `_loaded_cluster` 标记避免重复加载。

handler 是 sync `def`,K8s lib 是 async —— `run_k8s(coro)` 用 `asyncio.run` 同步包装。
仅在 sync handler 上下文调用(recovery router 全 sync def,FastAPI threadpool 无 running loop)。
"""
from __future__ import annotations

import asyncio
import logging
import os
from typing import Any, Optional

from app.config import settings


logger = logging.getLogger(__name__)

# 模块级标记:已加载哪个集群的 config(kubernetes_asyncio 全局状态)
_loaded_cluster: Optional[str] = None


async def ensure_kube_loaded(cluster_id: Optional[str] = None) -> None:
    """加载 kubeconfig(幂等)。优先 settings.kubeconfigs[cluster_id] 路径。

    cluster_id None → 用 settings.active_cluster。
    """
    global _loaded_cluster
    cluster = cluster_id or settings.active_cluster
    if _loaded_cluster == cluster:
        return

    from kubernetes_asyncio import config as k8s_config

    kubeconfig_path = settings.kubeconfigs.get(cluster, "")

    try:
        if kubeconfig_path and os.path.exists(kubeconfig_path):
            await k8s_config.load_kube_config(config_file=kubeconfig_path)
            logger.info("k8s_client loaded kubeconfig from %s (cluster=%s)", kubeconfig_path, cluster)
        else:
            # 兜底:in-cluster(SA)或默认 ~/.kube/config
            try:
                k8s_config.load_incluster_config()
                logger.info("k8s_client loaded in-cluster config (cluster=%s)", cluster)
            except Exception:
                await k8s_config.load_kube_config()
                logger.info("k8s_client loaded default kubeconfig ~/.kube/config (cluster=%s)", cluster)
    except Exception as e:
        logger.error("k8s_client failed to load kubeconfig: %s", e)
        raise

    _loaded_cluster = cluster


def reset_loaded_cluster() -> None:
    """测试用 — 清除已加载标记,强制下次重新加载。"""
    global _loaded_cluster
    _loaded_cluster = None


async def get_k8s_apps_api():
    """返回 AppsV1Api(Deployment / StatefulSet / DaemonSet 操作)。短生命周期,用完即关。"""
    from kubernetes_asyncio import client
    from kubernetes_asyncio.client.api_client import ApiClient

    await ensure_kube_loaded()
    api = ApiClient()
    return api, client.AppsV1Api(api)


async def get_k8s_core_api():
    """返回 CoreV1Api(Pod / Service / Secret / Node 操作)。短生命周期,用完即关。"""
    from kubernetes_asyncio import client
    from kubernetes_asyncio.client.api_client import ApiClient

    await ensure_kube_loaded()
    api = ApiClient()
    return api, client.CoreV1Api(api)


def run_k8s(coro) -> Any:
    """同步包装 async K8s 调用。

    recovery handler 是 sync `def`,在 FastAPI threadpool 里跑(无 running loop),
    故 `asyncio.run` 安全。若误在 async 上下文调,降级到 run_coroutine_threadsafe
    + 新 loop 兜底(避免 "already running loop" 崩)。
    """
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        # 无 running loop —— 正常路径
        return asyncio.run(coro)

    # 已有 running loop(理论上 recovery 不该走到这)—— 用新线程跑
    import threading
    result_box: dict[str, Any] = {}

    def _runner():
        try:
            result_box["value"] = asyncio.run(coro)
        except Exception as e:  # noqa: BLE001
            result_box["error"] = e

    t = threading.Thread(target=_runner)
    t.start()
    t.join()
    if "error" in result_box:
        raise result_box["error"]
    return result_box.get("value")


def k8s_ref(target_id: str) -> tuple[str, str]:
    """从 DSS 节点 properties 读 (namespace, name)。mapper 已写入这两个字段。

    缺失 → 抛 ValueError(handler 捕获返 success=False)。
    """
    from app.datasource.store import store

    node = store.get_node(target_id)
    if node is None:
        raise ValueError(f"target not found: {target_id}")
    props = node.properties or {}
    namespace = props.get("namespace") or settings.k8s_namespace
    name = props.get("name") or target_id.rsplit(":", 1)[-1]
    if not namespace or not name:
        raise ValueError(f"cannot derive (namespace, name) from target {target_id}")
    return namespace, name
