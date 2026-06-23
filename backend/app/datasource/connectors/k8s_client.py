"""K8s 客户端工厂 — PRD-001 Phase 2(余项) recovery handler 专用。

供 recovery handler 真实模式调 K8s API。**不复用给 connector** —— connector 是
多集群(每实例一个 cluster,实例级 `_k8s_loaded`),本工厂是 **switch-and-reload
跨集群**(模块级 `_active_cluster` 标记,切换时 reset + reload kubeconfig)。

`kubernetes_asyncio` 的 config 加载是**全局状态**(load_kube_config 写入模块级
configuration),故无法真正并发持多集群;本期接受 ~100ms 切换代价。Phase 3 上
per-ApiClient `Configuration` 走真并发。

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

# 模块级标记:当前 kubernetes_asyncio 全局 configuration 持有哪个集群
_active_cluster: Optional[str] = None


async def ensure_kube_loaded(cluster_id: Optional[str] = None) -> str:
    """加载 kubeconfig(幂等)。若当前已加载的集群 ≠ 目标集群 → 重新 load。

    cluster_id None → 用 settings.active_cluster。
    返回实际加载的 cluster_id(供调用方记录)。
    """
    global _active_cluster
    cluster = cluster_id or settings.active_cluster
    if _active_cluster == cluster:
        return cluster

    from kubernetes_asyncio import config as k8s_config

    kubeconfig_path = settings.kubeconfigs.get(cluster, "")

    try:
        if kubeconfig_path and os.path.exists(kubeconfig_path):
            await k8s_config.load_kube_config(config_file=kubeconfig_path)
            logger.info(
                "k8s_client loaded kubeconfig from %s (cluster=%s, prev=%s)",
                kubeconfig_path, cluster, _active_cluster,
            )
        else:
            # 兜底:in-cluster(SA)或默认 ~/.kube/config
            try:
                k8s_config.load_incluster_config()
                logger.info("k8s_client loaded in-cluster config (cluster=%s)", cluster)
            except Exception:
                await k8s_config.load_kube_config()
                logger.info("k8s_client loaded default kubeconfig ~/.kube/config (cluster=%s)", cluster)
    except Exception as e:
        logger.error("k8s_client failed to load kubeconfig for cluster %s: %s", cluster, e)
        raise

    _active_cluster = cluster
    return cluster


def reset_loaded_cluster() -> None:
    """测试用 — 清除已加载标记,强制下次重新加载。"""
    global _active_cluster
    _active_cluster = None


def get_active_cluster() -> Optional[str]:
    """测试 / 诊断用 — 当前 kubernetes_asyncio 全局 configuration 持有的集群。"""
    return _active_cluster


async def get_k8s_apps_api(cluster_id: Optional[str] = None):
    """返回 AppsV1Api(Deployment / StatefulSet / DaemonSet 操作)。短生命周期。

    cluster_id 决定路由到哪个 kubeconfig。None → settings.active_cluster。
    """
    from kubernetes_asyncio import client
    from kubernetes_asyncio.client.api_client import ApiClient

    await ensure_kube_loaded(cluster_id)
    api = ApiClient()
    return api, client.AppsV1Api(api)


async def get_k8s_core_api(cluster_id: Optional[str] = None):
    """返回 CoreV1Api(Pod / Service / Secret / Node 操作)。短生命周期。

    cluster_id 决定路由到哪个 kubeconfig。None → settings.active_cluster。
    """
    from kubernetes_asyncio import client
    from kubernetes_asyncio.client.api_client import ApiClient

    await ensure_kube_loaded(cluster_id)
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


def resolve_cluster_id(target_id: str) -> str:
    """从 target_id 解析所属集群:

    1. 优先读 DSS 节点 properties["cluster_id"]
    2. 兜底解析 target_id 第二段(约定格式 `<type>:<cluster>:<ns>:<name>`)
    3. 二者都缺 → settings.active_cluster

    若 settings.kubeconfigs 非空且解析出的 cluster 不在其中 → ValueError(
    防止误把动作派到没配 kubeconfig 的集群)。kubeconfigs 为空时(测试 / 单集群)
    跳过校验,直接返回。
    """
    from app.datasource.store import store

    cluster: Optional[str] = None
    node = store.get_node(target_id)
    if node is not None:
        props = node.properties or {}
        cluster = props.get("cluster_id") or None

    if not cluster:
        # 兜底:约定 target_id 第二段是集群名
        parts = target_id.split(":")
        if len(parts) >= 2 and parts[1]:
            cluster = parts[1]

    if not cluster:
        cluster = settings.active_cluster

    if settings.kubeconfigs and cluster not in settings.kubeconfigs:
        raise ValueError(
            f"unknown cluster '{cluster}' for target {target_id}; "
            f"configured kubeconfigs: {list(settings.kubeconfigs.keys())}"
        )
    return cluster


def k8s_ref(target_id: str) -> tuple[str, str, str]:
    """从 DSS 节点 properties 读 (cluster_id, namespace, name)。

    cluster_id 通过 `resolve_cluster_id` 派生(支持 DSS prop / target_id 兜底)。
    namespace / name 从 properties 读(connector mapper 写入),缺失抛 ValueError。
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

    cluster_id = resolve_cluster_id(target_id)
    return cluster_id, namespace, name
