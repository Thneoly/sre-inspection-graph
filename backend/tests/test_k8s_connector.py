"""K8sConnector 单元测试 — PRD-004 Sprint 1。

策略:不真接 K8s 集群。用 monkeypatch 把 sync_once 内部使用的
kubernetes_asyncio API 客户端替换成 mock,验证:
- DSS 写入 / diff / 删除消失节点的逻辑
- BaseConnector 的 status / sync_count / 错误吞噬
- ConnectorRegistry / 控制端点

mapper 内部逻辑已在 test_k8s_mapper.py 充分覆盖,这里聚焦在
"K8s API → DSS 写入" 的胶水代码。
"""

from __future__ import annotations

import asyncio
from types import SimpleNamespace
from typing import Any

import pytest

from app.datasource.connectors.base import BaseConnector, SyncResult
from app.datasource.connectors.k8s_connector import (
    K8sConnector,
    _to_ref,
    _index_rs_to_deploy,
    _find_pod_deployment,
)
from app.datasource.connectors.sync_orchestrator import ConnectorRegistry
from app.datasource.store import store


# ============================================================
# Fixture
# ============================================================

@pytest.fixture(autouse=True)
def _clear_dss_for_connector():
    """每个测试前清掉 connector 创建过的节点/边。"""
    to_del_n = [nid for nid, n in store.nodes.items()
                if (n.properties or {}).get("discovery_method") == "k8s_connector"]
    for nid in to_del_n:
        del store.nodes[nid]
    to_del_e = [eid for eid, e in store.edges.items()
                if (e.properties or {}).get("discovery_method") == "k8s_connector"]
    for eid in to_del_e:
        del store.edges[eid]
    yield


# ============================================================
# Mock K8s objects
# ============================================================

def _mk_metadata(name, namespace="otel-demo", uid="", labels=None, owner_refs=None):
    return SimpleNamespace(
        name=name,
        namespace=namespace,
        uid=uid or f"uid-{name}",
        labels=labels or {},
        annotations={},
        owner_references=owner_refs or [],
    )


def _mk_owner_ref(kind, name, uid):
    return SimpleNamespace(kind=kind, name=name, uid=uid)


class _FakeApiList:
    def __init__(self, items):
        self.items = items


# ============================================================
# Tests
# ============================================================

class TestToRef:
    def test_basic_conversion(self):
        obj = SimpleNamespace(
            metadata=_mk_metadata("frontend"),
            spec=None,
            status=None,
        )
        ref = _to_ref(obj, kind="Deployment")
        assert ref.name == "frontend"
        assert ref.namespace == "otel-demo"
        assert ref.kind == "Deployment"

    def test_owner_deployment_injected_into_spec(self):
        obj = SimpleNamespace(
            metadata=_mk_metadata("pod-1"),
            spec=SimpleNamespace(to_dict=lambda: {"node_name": "vm2"}),
            status=None,
        )
        ref = _to_ref(obj, kind="Pod", owner_deployment="otel-demo-frontend")
        assert ref.spec["_owner_deployment"] == "otel-demo-frontend"
        assert ref.spec["nodeName"] == "vm2"  # snake → camel

    def test_configmap_data_lifted_to_spec(self):
        obj = SimpleNamespace(
            metadata=_mk_metadata("flagd-config"),
            spec=None,
            status=None,
            data={"flag.json": "{}"},
        )
        ref = _to_ref(obj, kind="ConfigMap")
        assert ref.spec["data"] == {"flag.json": "{}"}


class TestRsToDeploy:
    def test_replicaset_owner_indexed(self):
        rs1 = SimpleNamespace(metadata=_mk_metadata(
            "rs-1", uid="rs-uid-1",
            owner_refs=[_mk_owner_ref("Deployment", "otel-demo-frontend", "dep-uid-1")],
        ))
        rs2 = SimpleNamespace(metadata=_mk_metadata("rs-2", uid="rs-uid-2", owner_refs=[]))
        index = _index_rs_to_deploy([rs1, rs2])
        assert index == {"rs-uid-1": "otel-demo-frontend"}

    def test_pod_to_deployment_via_rs(self):
        rs_index = {"rs-uid-1": "otel-demo-frontend"}
        pod = SimpleNamespace(metadata=_mk_metadata(
            "frontend-x",
            owner_refs=[_mk_owner_ref("ReplicaSet", "rs-1", "rs-uid-1")],
        ))
        assert _find_pod_deployment(pod, rs_index) == "otel-demo-frontend"

    def test_pod_no_owner_returns_empty(self):
        pod = SimpleNamespace(metadata=_mk_metadata("orphan", owner_refs=[]))
        assert _find_pod_deployment(pod, {}) == ""


class TestK8sConnectorWriteToDss:
    """直接测 _write_to_dss — 不需要 mock K8s 客户端。"""

    def test_initial_write_adds_nodes_and_edges(self):
        c = K8sConnector(cluster_id="vm-cluster", namespace="otel-demo")
        nodes = [
            {"id": "deploy:vm-cluster:otel-demo:frontend", "type": "Deployment", "name": "frontend",
             "properties": {"discovery_method": "k8s_connector", "image": "v1"}},
            {"id": "pod:vm-cluster:otel-demo:frontend-x", "type": "Pod", "name": "frontend-x",
             "properties": {"discovery_method": "k8s_connector", "phase": "Running"}},
        ]
        edges = [
            {"id": "e-1", "source_id": "deploy:vm-cluster:otel-demo:frontend",
             "target_id": "pod:vm-cluster:otel-demo:frontend-x",
             "relationship_type": "CONTAINS", "relationship_name": "包含",
             "properties": {"discovery_method": "k8s_connector"}},
        ]
        result = c._write_to_dss(nodes, edges)
        assert result.nodes_added == 2
        assert result.nodes_removed == 0
        assert result.edges_added == 1
        assert store.get_node("deploy:vm-cluster:otel-demo:frontend").type == "Deployment"

    def test_second_write_no_changes_is_noop(self):
        c = K8sConnector()
        nodes = [
            {"id": "deploy:vm-cluster:otel-demo:frontend", "type": "Deployment", "name": "frontend",
             "properties": {"discovery_method": "k8s_connector", "image": "v1"}},
        ]
        c._write_to_dss(nodes, [])
        result = c._write_to_dss(nodes, [])
        assert result.nodes_added == 0
        assert result.nodes_updated == 0
        assert result.nodes_removed == 0

    def test_property_change_counts_as_update(self):
        c = K8sConnector()
        nodes_v1 = [
            {"id": "deploy:vm-cluster:otel-demo:frontend", "type": "Deployment", "name": "frontend",
             "properties": {"discovery_method": "k8s_connector", "image": "v1"}},
        ]
        nodes_v2 = [
            {"id": "deploy:vm-cluster:otel-demo:frontend", "type": "Deployment", "name": "frontend",
             "properties": {"discovery_method": "k8s_connector", "image": "v2"}},
        ]
        c._write_to_dss(nodes_v1, [])
        result = c._write_to_dss(nodes_v2, [])
        assert result.nodes_updated == 1
        assert store.get_node("deploy:vm-cluster:otel-demo:frontend").properties["image"] == "v2"

    def test_disappeared_node_removed(self):
        c = K8sConnector()
        nodes_v1 = [
            {"id": "pod:vm-cluster:otel-demo:gone", "type": "Pod", "name": "gone",
             "properties": {"discovery_method": "k8s_connector"}},
            {"id": "pod:vm-cluster:otel-demo:keeper", "type": "Pod", "name": "keeper",
             "properties": {"discovery_method": "k8s_connector"}},
        ]
        nodes_v2 = [nodes_v1[1]]  # gone 消失
        c._write_to_dss(nodes_v1, [])
        result = c._write_to_dss(nodes_v2, [])
        assert result.nodes_removed == 1
        assert store.get_node("pod:vm-cluster:otel-demo:gone") is None
        assert store.get_node("pod:vm-cluster:otel-demo:keeper") is not None

    def test_baseline_node_not_touched(self):
        """connector 不动其他来源的节点(没 discovery_method=k8s_connector)。"""
        # 模拟 baseline 节点
        from app.datasource.models import DataNode
        baseline = DataNode(
            id="pod:cce-prod-01:order:order-api",
            type="Pod",
            name="order-api",
            properties={"discovery_method": "neo4j_baseline"},
        )
        store.upsert_node(baseline)

        c = K8sConnector()
        # connector 这一轮没拉到任何 OTel pod
        result = c._write_to_dss([], [])
        assert result.nodes_removed == 0
        # baseline 还在
        assert store.get_node("pod:cce-prod-01:order:order-api") is not None

        # cleanup
        del store.nodes["pod:cce-prod-01:order:order-api"]


class TestBaseConnector:
    def test_sync_once_called_at_start(self):
        class Counter(BaseConnector):
            name = "counter"
            sync_interval_seconds = 60  # 大值,确保只跑第一次

            def __init__(self):
                super().__init__()
                self.count = 0

            async def sync_once(self):
                self.count += 1
                return SyncResult(notes=[f"run-{self.count}"])

        async def run():
            c = Counter()
            await c.start()
            await asyncio.sleep(0.05)  # 让 _run_once 跑完
            await c.stop()
            return c

        c = asyncio.run(run())
        assert c.count == 1
        assert c.status()["sync_count"] >= 1

    def test_exception_swallowed(self):
        class Boom(BaseConnector):
            name = "boom"
            sync_interval_seconds = 60

            async def sync_once(self):
                raise RuntimeError("k8s api down")

        async def run():
            c = Boom()
            await c.start()
            await asyncio.sleep(0.05)
            await c.stop()
            return c

        c = asyncio.run(run())
        s = c.status()
        assert "k8s api down" in s["last_error_message"]
        assert s["error_count_24h"] >= 1

    def test_trigger_sync_now(self):
        class Once(BaseConnector):
            name = "once"
            sync_interval_seconds = 9999  # 不会触发循环

            async def sync_once(self):
                return SyncResult(nodes_added=5)

        result = asyncio.run(Once().trigger_sync_now())
        assert result.nodes_added == 5


class TestConnectorRegistry:
    def test_register_and_get(self):
        reg = ConnectorRegistry()
        c1 = K8sConnector()
        reg.register(c1)
        assert reg.get("k8s") is c1
        assert reg.names() == ["k8s"]

    def test_overwrite_warning(self):
        reg = ConnectorRegistry()
        reg.register(K8sConnector())
        reg.register(K8sConnector())  # 不抛错,只 log warning
        assert len(reg.all()) == 1


class TestConnectorAPI:
    """端点级测试,通过 TestClient。"""

    def test_status_empty(self, client_no_autostart):
        r = client_no_autostart.get("/api/v1/connectors/status")
        assert r.status_code == 200
        assert r.json() == {"connectors": [], "total": 0}

    def test_get_unknown_connector_404(self, client_no_autostart):
        r = client_no_autostart.get("/api/v1/connectors/nope")
        assert r.status_code == 404

    def test_sync_now_unknown_connector_404(self, client_no_autostart):
        r = client_no_autostart.post("/api/v1/connectors/nope/sync-now")
        assert r.status_code == 404

    def test_status_after_register(self, client_no_autostart, monkeypatch):
        from app.datasource.connectors.sync_orchestrator import registry
        # 清掉已有再注册一个
        registry._connectors.clear()
        registry.register(K8sConnector(cluster_id="test", namespace="ns"))
        r = client_no_autostart.get("/api/v1/connectors/status")
        body = r.json()
        assert body["total"] == 1
        assert body["connectors"][0]["name"] == "k8s"
        registry._connectors.clear()


# ============================================================
# fixture for API tests
# ============================================================

@pytest.fixture
def client_no_autostart(monkeypatch):
    """禁用 connectors 自动启动,加载 app。"""
    monkeypatch.setenv("CONNECTORS_AUTOSTART", "0")
    # 重新加载 settings
    from app import config as cfg_module
    monkeypatch.setattr(cfg_module.settings, "connectors_autostart", False)
    from fastapi.testclient import TestClient
    from app.main import app
    with TestClient(app) as c:
        yield c
