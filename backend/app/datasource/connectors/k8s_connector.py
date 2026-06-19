"""K8sConnector — 从 Kubernetes API 拉拓扑同步到 DSS。

工作流:
1. 启动时 load_kube_config(支持 KUBECONFIGS / kubeconfig path / in-cluster ServiceAccount)
2. sync_once 每 30 秒:
   - 并发拉:Deployment / Pod / Service / Node / ConfigMap / Secret(namespace 限定)
   - 转 K8sObjectRef → 调 k8s_mapper.map_all → 拿到 nodes/edges 列表
   - 与 DSS 现有内容做 diff,upsert 新增/修改、删除消失的(只删 discovery_method=k8s_connector 的)
   - 返回 SyncResult

设计要点:
- ReplicaSet 作为中间层不入图 — 直接 Pod ownerReferences → ReplicaSet → Deployment 反推。
- Pod owner 推导优先用 ReplicaSet uid 匹配,失败再用 labels 兜底(见 mapper.py)。
- 删节点策略保守:只删本 connector 创建的(看 properties.discovery_method),
  不动 baseline 静态拓扑。这让 Sprint 1 与现有 mock 数据可共存。
"""

from __future__ import annotations

import asyncio
import logging
import os
from typing import Any, Optional

from app.config import settings
from app.datasource.connectors.base import BaseConnector, SyncResult
from app.datasource.connectors.k8s_mapper import (
    K8sObjectRef,
    MapperInput,
    extract_owner_nodes,
    map_all,
)
from app.datasource.models import DataEdge, DataNode
from app.datasource.store import store


logger = logging.getLogger(__name__)


class K8sConnector(BaseConnector):
    """K8s API → DSS 同步。"""

    name = "k8s"

    def __init__(
        self,
        cluster_id: Optional[str] = None,
        namespace: Optional[str] = None,
        kubeconfig_path: Optional[str] = None,
        sync_interval_seconds: Optional[int] = None,
        release_prefix: str = "otel-demo",
    ):
        super().__init__()
        self.cluster_id = cluster_id or settings.active_cluster
        self.namespace = namespace or settings.k8s_namespace
        self.kubeconfig_path = kubeconfig_path or settings.kubeconfigs.get(self.cluster_id, "")
        self.sync_interval_seconds = sync_interval_seconds or settings.k8s_sync_interval_seconds
        self.release_prefix = release_prefix
        self._k8s_loaded = False

    # ============================================================
    # 主同步流程
    # ============================================================

    async def sync_once(self) -> SyncResult:
        await self._ensure_kube_loaded()

        # 拉 K8s 对象 — 异步并发
        from kubernetes_asyncio import client  # lazy import,避免测试 mock 时被影响
        from kubernetes_asyncio.client.api_client import ApiClient

        async with ApiClient() as api:
            core = client.CoreV1Api(api)
            apps = client.AppsV1Api(api)
            ns = self.namespace
            (deployments, replicasets, pods, services,
             nodes, configmaps, secrets) = await asyncio.gather(
                apps.list_namespaced_deployment(ns),
                apps.list_namespaced_replica_set(ns),
                core.list_namespaced_pod(ns),
                core.list_namespaced_service(ns),
                core.list_node(),
                core.list_namespaced_config_map(ns),
                core.list_namespaced_secret(ns),
            )

        # 转 K8sObjectRef 形态(纯数据,可被 mapper 处理)
        rs_to_deploy = _index_rs_to_deploy(replicasets.items)
        mapper_input = MapperInput(
            cluster_id=self.cluster_id,
            namespace=self.namespace,
            release_prefix=self.release_prefix,
            nodes=[_to_ref(n, kind="Node") for n in nodes.items],
            deployments=[_to_ref(d, kind="Deployment") for d in deployments.items],
            pods=[_to_ref(p, kind="Pod", owner_deployment=_find_pod_deployment(p, rs_to_deploy)) for p in pods.items],
            services=[_to_ref(s, kind="Service") for s in services.items],
            configmaps=[_to_ref(c, kind="ConfigMap") for c in configmaps.items],
            secrets=[_to_ref(s, kind="Secret") for s in secrets.items],
        )

        # 调 mapper
        node_dicts, edge_dicts = map_all(mapper_input)
        # 把 _component_owner / _middleware_owner 抽出来,合并到 nodes
        owner_nodes = extract_owner_nodes(node_dicts)
        node_dicts = node_dicts + owner_nodes

        # 写 DSS(diff + upsert + 删消失的)
        result = self._write_to_dss(node_dicts, edge_dicts)
        result.notes.append(f"namespace={self.namespace} cluster={self.cluster_id}")
        return result

    # ============================================================
    # K8s 客户端初始化
    # ============================================================

    async def _ensure_kube_loaded(self):
        if self._k8s_loaded:
            return
        from kubernetes_asyncio import config as k8s_config

        try:
            if self.kubeconfig_path and os.path.exists(self.kubeconfig_path):
                await k8s_config.load_kube_config(config_file=self.kubeconfig_path)
                logger.info("k8s connector loaded kubeconfig from %s", self.kubeconfig_path)
            else:
                # 兜底:in-cluster(ServiceAccount)或默认 ~/.kube/config
                try:
                    k8s_config.load_incluster_config()
                    logger.info("k8s connector loaded in-cluster config")
                except Exception:
                    await k8s_config.load_kube_config()
                    logger.info("k8s connector loaded default kubeconfig (~/.kube/config)")
        except Exception as e:
            logger.error("k8s connector failed to load kubeconfig: %s", e)
            raise
        self._k8s_loaded = True

    # ============================================================
    # DSS diff & write
    # ============================================================

    def _write_to_dss(self, node_dicts: list[dict], edge_dicts: list[dict]) -> SyncResult:
        result = SyncResult()

        new_node_ids: set[str] = {n["id"] for n in node_dicts}
        new_edge_ids: set[str] = {e["id"] for e in edge_dicts}

        # 现有 connector 创建的节点 / 边
        existing_node_ids = {
            n.id for n in store.get_all_nodes()
            if (n.properties or {}).get("discovery_method") == "k8s_connector"
        }
        existing_edge_ids = {
            e.id for e in store.get_all_edges()
            if (e.properties or {}).get("discovery_method") == "k8s_connector"
        }

        # Upsert nodes
        for nd in node_dicts:
            existing = store.get_node(nd["id"])
            node = DataNode(
                id=nd["id"],
                type=nd["type"],
                name=nd["name"],
                properties=nd.get("properties", {}),
            )
            store.upsert_node(node)
            if existing is None:
                result.nodes_added += 1
            else:
                # 简单认为有变更(可优化为深度对比);多数情况下 image/replicas/phase 在变。
                if existing.properties != node.properties:
                    result.nodes_updated += 1

        # Remove disappeared nodes(只删自己创建的)
        to_remove_nodes = existing_node_ids - new_node_ids
        for nid in to_remove_nodes:
            if nid in store.nodes:
                del store.nodes[nid]
                result.nodes_removed += 1

        # Upsert edges
        for ed in edge_dicts:
            existing = store.get_edge(ed["id"])
            edge = DataEdge(
                id=ed["id"],
                source_id=ed["source_id"],
                target_id=ed["target_id"],
                relationship_type=ed["relationship_type"],
                relationship_name=ed.get("relationship_name", ""),
                properties=ed.get("properties", {}),
            )
            store.upsert_edge(edge)
            if existing is None:
                result.edges_added += 1
            elif existing.properties != edge.properties:
                result.edges_updated += 1

        # Remove disappeared edges
        to_remove_edges = existing_edge_ids - new_edge_ids
        for eid in to_remove_edges:
            if eid in store.edges:
                del store.edges[eid]
                result.edges_removed += 1

        return result


# ============================================================
# K8s 对象转换
# ============================================================

def _to_ref(obj: Any, kind: str, owner_deployment: str = "") -> K8sObjectRef:
    """kubernetes-asyncio 模型对象 → 我们的 K8sObjectRef。

    每个 K8s model 对象有:
      - metadata.name / namespace / labels / annotations / uid
      - spec / status(具体子字段视 kind 而定)
    """
    md = obj.metadata
    spec = _to_dict(getattr(obj, "spec", None))
    status = _to_dict(getattr(obj, "status", None))
    if owner_deployment:
        spec["_owner_deployment"] = owner_deployment
    # ConfigMap 的 data 在顶层,不在 spec
    if kind == "ConfigMap":
        spec["data"] = getattr(obj, "data", {}) or {}
    if kind == "Secret":
        # Secret 的 type 在顶层
        spec["type"] = getattr(obj, "type", "Opaque")
    return K8sObjectRef(
        name=md.name,
        namespace=getattr(md, "namespace", "") or "",
        kind=kind,
        labels=dict(md.labels or {}),
        annotations=dict(md.annotations or {}),
        spec=spec,
        status=status,
        uid=md.uid or "",
    )


def _to_dict(obj: Any) -> dict:
    """K8s model → dict(递归)。kubernetes-asyncio 自带 to_dict 但 key 用驼峰,我们转回。"""
    if obj is None:
        return {}
    if hasattr(obj, "to_dict"):
        return _camel_to_keep(obj.to_dict())
    if isinstance(obj, dict):
        return obj
    return {}


def _camel_to_keep(d: Any) -> Any:
    """kubernetes-asyncio.to_dict() 把 K8s API 的 camelCase 转 snake_case;
    但我们 mapper 里用 K8s 风格的 camelCase(nodeName / podIP / containerStatuses)
    所以这里转回来。

    简化做法:常见 K8s 字段映射表 + 递归。
    """
    if isinstance(d, dict):
        new = {}
        for k, v in d.items():
            new_key = SNAKE_TO_CAMEL.get(k, k)
            new[new_key] = _camel_to_keep(v)
        return new
    if isinstance(d, list):
        return [_camel_to_keep(x) for x in d]
    return d


# K8s API 常见字段:snake_case → camelCase 映射(只列我们 mapper 用到的)。
SNAKE_TO_CAMEL: dict[str, str] = {
    "node_name": "nodeName",
    "pod_ip": "podIP",
    "container_statuses": "containerStatuses",
    "ready_replicas": "readyReplicas",
    "config_map": "configMap",
    "secret_name": "secretName",
    "config_map_ref": "configMapRef",
    "secret_ref": "secretRef",
    "env_from": "envFrom",
    "cluster_ip": "clusterIP",
}


def _index_rs_to_deploy(replicasets: list[Any]) -> dict[str, str]:
    """ReplicaSet uid → owning Deployment 名。"""
    out: dict[str, str] = {}
    for rs in replicasets:
        for ref in (rs.metadata.owner_references or []):
            if ref.kind == "Deployment":
                out[rs.metadata.uid] = ref.name
                break
    return out


def _find_pod_deployment(pod: Any, rs_to_deploy: dict[str, str]) -> str:
    """Pod ownerReferences.ReplicaSet → 反查 Deployment 名。"""
    for ref in (pod.metadata.owner_references or []):
        if ref.kind == "ReplicaSet":
            return rs_to_deploy.get(ref.uid, "")
    return ""
