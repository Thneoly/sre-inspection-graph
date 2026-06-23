"""k8s_mapper 单元测试 — PRD-004 Sprint 1。

纯函数 mapper,用 dataclass 直接构造输入,不依赖 kubernetes-asyncio。
"""

from __future__ import annotations

import pytest

from app.datasource.connectors.k8s_mapper import (
    INFRA_NAMES,
    K8sObjectRef,
    MapperInput,
    detect_middleware,
    extract_owner_nodes,
    is_infra,
    map_all,
    normalize_component_name,
)


# ============================================================
# helper
# ============================================================

def _ref(name, kind, labels=None, spec=None, status=None, namespace="otel-demo", uid=""):
    return K8sObjectRef(
        name=name,
        namespace=namespace,
        kind=kind,
        labels=labels or {},
        annotations={},
        spec=spec or {},
        status=status or {},
        uid=uid or f"uid-{name}",
    )


def _deploy(name, replicas=1, image="img:1.0"):
    return _ref(
        name, "Deployment",
        spec={
            "replicas": replicas,
            "template": {"spec": {"containers": [{"name": "app", "image": image}]}},
        },
        status={"readyReplicas": replicas},
    )


def _pod(name, owner_deploy, node_name="vm2", labels=None, volumes=None, env_from=None, ready=True):
    spec = {
        "_owner_deployment": owner_deploy,
        "nodeName": node_name,
        "volumes": volumes or [],
        "containers": [{"name": "app", "image": "img:1.0", "envFrom": env_from or []}],
    }
    status = {
        "phase": "Running",
        "podIP": "10.244.0.1",
        "conditions": [{"type": "Ready", "status": "True" if ready else "False"}],
        "containerStatuses": [{"restartCount": 0}],
    }
    return _ref(name, "Pod", labels=labels or {"app.kubernetes.io/name": owner_deploy},
                spec=spec, status=status)


def _svc(name, selector, port=8080):
    return _ref(name, "Service", spec={
        "selector": selector,
        "ports": [{"port": port}],
        "clusterIP": "10.96.0.1",
        "type": "ClusterIP",
    })


def _node(name, internal_ip="192.168.56.5"):
    return _ref(name, "Node", namespace="", spec={}, status={
        "capacity": {"cpu": "3", "memory": "5440Mi", "pods": "110"},
        "addresses": [{"type": "InternalIP", "address": internal_ip}],
    })


def _cm(name, data=None):
    return _ref(name, "ConfigMap", spec={"data": data or {"key": "v"}})


def _secret(name, secret_type="Opaque"):
    return _ref(name, "Secret", spec={"type": secret_type})


# ============================================================
# 命名规则
# ============================================================

class TestNormalizeComponentName:
    def test_strips_release_prefix(self):
        assert normalize_component_name("otel-demo-frontend") == "frontend"

    def test_strips_service_suffix(self):
        assert normalize_component_name("otel-demo-cartservice") == "cart"
        assert normalize_component_name("otel-demo-paymentservice") == "payment"

    def test_keeps_short_names(self):
        # ad 是真实 OTel 服务名,不能被 service 后缀逻辑误吃
        assert normalize_component_name("otel-demo-adservice") == "ad"

    def test_known_split_names(self):
        assert normalize_component_name("otel-demo-frauddetectionservice") == "fraud-detection"
        assert normalize_component_name("otel-demo-productcatalogservice") == "product-catalog"
        assert normalize_component_name("otel-demo-frontendproxy") == "frontend-proxy"

    def test_no_release_prefix(self):
        assert normalize_component_name("standalone-svc") == "standalone-svc"

    def test_custom_prefix(self):
        assert normalize_component_name("foo-cartservice", release_prefix="foo") == "cart"


class TestDetectMiddleware:
    def test_valkey_to_redis(self):
        assert detect_middleware("otel-demo-valkey") == ("Redis", "redis")

    def test_kafka(self):
        assert detect_middleware("otel-demo-kafka") == ("Kafka", "kafka")

    def test_postgres(self):
        assert detect_middleware("otel-demo-postgres") == ("PostgreSQL", "postgres")
        assert detect_middleware("otel-demo-postgresql") == ("PostgreSQL", "postgres")

    def test_mysql(self):
        assert detect_middleware("otel-demo-mysql") == ("MySQL", "mysql")

    def test_substring_match(self):
        # otel-demo-valkey-cart 也算 valkey
        assert detect_middleware("otel-demo-valkey-cart") == ("Redis", "redis")

    def test_business_service_not_middleware(self):
        assert detect_middleware("otel-demo-frontend") is None
        assert detect_middleware("otel-demo-checkoutservice") is None


class TestIsInfra:
    def test_infra_names(self):
        for n in ("loadgenerator", "otelcol", "jaeger", "prometheus-server"):
            assert is_infra(f"otel-demo-{n}") is True, n

    def test_business_not_infra(self):
        assert is_infra("otel-demo-frontend") is False
        assert is_infra("otel-demo-cartservice") is False

    def test_middleware_not_infra(self):
        # 中间件不属于 infra,会被识别为 Redis/Kafka/...,挂在 Application 之外
        assert is_infra("otel-demo-valkey") is False


# ============================================================
# 主 mapper 流程
# ============================================================

class TestMapAll:
    def test_minimal_business_service_full_chain(self):
        """1 deployment + 1 pod + 1 service + 1 node → 完整 app/comp/deploy/pod/svc 链。"""
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            nodes=[_node("vm2")],
            deployments=[_deploy("otel-demo-frontend")],
            pods=[_pod("otel-demo-frontend-abc", "otel-demo-frontend",
                       labels={"app.kubernetes.io/name": "otel-demo-frontend"})],
            services=[_svc("otel-demo-frontend",
                           selector={"app.kubernetes.io/name": "otel-demo-frontend"})],
        )
        nodes, edges = map_all(inp)

        # 把 owner 节点合上来再断言
        nodes = nodes + extract_owner_nodes(nodes)
        ids = {n["id"] for n in nodes}
        assert "app:vm-cluster:otel-demo:otel-demo" in ids
        assert "comp:vm-cluster:otel-demo:frontend" in ids
        assert "deploy:vm-cluster:otel-demo:otel-demo-frontend" in ids
        assert "pod:vm-cluster:otel-demo:otel-demo-frontend-abc" in ids
        assert "svc:vm-cluster:otel-demo:otel-demo-frontend" in ids
        assert "node:vm-cluster:vm2" in ids

        rels = {(e["source_id"], e["relationship_type"], e["target_id"]) for e in edges}
        assert ("app:vm-cluster:otel-demo:otel-demo", "CONTAINS",
                "comp:vm-cluster:otel-demo:frontend") in rels
        assert ("comp:vm-cluster:otel-demo:frontend", "DEPLOYED_AS",
                "deploy:vm-cluster:otel-demo:otel-demo-frontend") in rels
        assert ("deploy:vm-cluster:otel-demo:otel-demo-frontend", "CONTAINS",
                "pod:vm-cluster:otel-demo:otel-demo-frontend-abc") in rels
        assert ("pod:vm-cluster:otel-demo:otel-demo-frontend-abc", "SCHEDULED_ON",
                "node:vm-cluster:vm2") in rels
        assert ("svc:vm-cluster:otel-demo:otel-demo-frontend", "ROUTES_TO",
                "pod:vm-cluster:otel-demo:otel-demo-frontend-abc") in rels

    def test_infra_pod_does_not_create_component(self):
        """loadgenerator 是 infra,应该入图 deployment+pod,但不创建 ApplicationComponent。"""
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-loadgenerator")],
            pods=[_pod("otel-demo-loadgenerator-x", "otel-demo-loadgenerator")],
        )
        nodes, edges = map_all(inp)
        nodes = nodes + extract_owner_nodes(nodes)
        ids = {n["id"] for n in nodes}
        assert "deploy:vm-cluster:otel-demo:otel-demo-loadgenerator" in ids
        assert "pod:vm-cluster:otel-demo:otel-demo-loadgenerator-x" in ids
        # 不应该有 component
        assert "comp:vm-cluster:otel-demo:loadgenerator" not in ids
        # app -CONTAINS-> comp:loadgenerator 这条边不应该存在
        bad = [e for e in edges
               if e["relationship_type"] == "CONTAINS"
               and e["source_id"].startswith("app:")
               and "loadgenerator" in e["target_id"]]
        assert bad == []

    def test_middleware_creates_redis_node_not_component(self):
        """valkey 是中间件 → Redis 节点,不是 ApplicationComponent。"""
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-valkey")],
            pods=[_pod("otel-demo-valkey-1", "otel-demo-valkey")],
        )
        nodes, edges = map_all(inp)
        nodes = nodes + extract_owner_nodes(nodes)

        redis_nodes = [n for n in nodes if n["type"] == "Redis"]
        assert len(redis_nodes) == 1
        assert redis_nodes[0]["id"] == "redis:vm-cluster:otel-demo:otel-demo-valkey"
        # 不应该有 valkey ApplicationComponent
        assert not any(n["type"] == "ApplicationComponent" and "valkey" in n["id"] for n in nodes)
        # Redis -DEPLOYED_AS-> Deployment 边
        rels = {(e["source_id"], e["relationship_type"]) for e in edges}
        assert ("redis:vm-cluster:otel-demo:otel-demo-valkey", "DEPLOYED_AS") in rels

    def test_postgres_creates_postgresql_node(self):
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-postgres")],
            pods=[_pod("otel-demo-postgres-1", "otel-demo-postgres")],
        )
        nodes, _ = map_all(inp)
        nodes = nodes + extract_owner_nodes(nodes)
        pg_nodes = [n for n in nodes if n["type"] == "PostgreSQL"]
        assert len(pg_nodes) == 1
        assert pg_nodes[0]["id"] == "postgres:vm-cluster:otel-demo:otel-demo-postgres"

    def test_pod_uses_configmap_via_volume(self):
        cm_vol = [{"name": "cfg", "configMap": {"name": "flagd-config"}}]
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-flagd")],
            pods=[_pod("otel-demo-flagd-1", "otel-demo-flagd", volumes=cm_vol)],
            configmaps=[_cm("flagd-config", data={"flags.json": "{}"})],
        )
        nodes, edges = map_all(inp)
        ids = {n["id"] for n in nodes}
        assert "configmap:vm-cluster:otel-demo:flagd-config" in ids
        rels = {(e["source_id"], e["relationship_type"], e["target_id"]) for e in edges}
        assert ("pod:vm-cluster:otel-demo:otel-demo-flagd-1", "USES",
                "configmap:vm-cluster:otel-demo:flagd-config") in rels

    def test_pod_uses_secret_via_envfrom(self):
        env_from = [{"secretRef": {"name": "db-creds"}}]
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-checkoutservice")],
            pods=[_pod("otel-demo-checkoutservice-1", "otel-demo-checkoutservice",
                       env_from=env_from)],
            secrets=[_secret("db-creds")],
        )
        nodes, edges = map_all(inp)
        ids = {n["id"] for n in nodes}
        assert "secret:vm-cluster:otel-demo:db-creds" in ids
        rels = {(e["source_id"], e["relationship_type"], e["target_id"]) for e in edges}
        assert ("pod:vm-cluster:otel-demo:otel-demo-checkoutservice-1", "USES",
                "secret:vm-cluster:otel-demo:db-creds") in rels

    def test_unreferenced_configmap_skipped(self):
        """没被任何 Pod 挂载的 ConfigMap 不入图,避免 kube-root-ca.crt 之类的污染。"""
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-frontend")],
            pods=[_pod("otel-demo-frontend-1", "otel-demo-frontend")],
            configmaps=[_cm("kube-root-ca.crt"), _cm("unused-cm")],
        )
        nodes, _ = map_all(inp)
        ids = {n["id"] for n in nodes}
        assert not any("kube-root-ca" in i for i in ids)
        assert not any("unused-cm" in i for i in ids)

    def test_service_routes_to_multiple_pods(self):
        sel = {"app.kubernetes.io/name": "otel-demo-frontend"}
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-frontend", replicas=3)],
            pods=[
                _pod("otel-demo-frontend-1", "otel-demo-frontend", labels=sel),
                _pod("otel-demo-frontend-2", "otel-demo-frontend", labels=sel),
                _pod("otel-demo-frontend-3", "otel-demo-frontend", labels=sel),
            ],
            services=[_svc("otel-demo-frontend", selector=sel)],
        )
        _, edges = map_all(inp)
        routes_to = [e for e in edges if e["relationship_type"] == "ROUTES_TO"]
        assert len(routes_to) == 3
        targets = {e["target_id"] for e in routes_to}
        assert all(t.startswith("pod:vm-cluster:otel-demo:otel-demo-frontend-") for t in targets)

    def test_service_with_no_matching_pods(self):
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[],
            pods=[],
            services=[_svc("orphan", selector={"app": "nothing"})],
        )
        nodes, edges = map_all(inp)
        # service 节点存在,但 ROUTES_TO 边 0 条
        ids = {n["id"] for n in nodes}
        assert "svc:vm-cluster:otel-demo:orphan" in ids
        assert not [e for e in edges if e["relationship_type"] == "ROUTES_TO"]

    def test_deployment_replicas_in_properties(self):
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-frontend", replicas=3, image="frontend:v2")],
        )
        nodes, _ = map_all(inp)
        dep_node = next(n for n in nodes if n["type"] == "Deployment")
        assert dep_node["properties"]["replicas_desired"] == 3
        assert dep_node["properties"]["replicas_ready"] == 3
        assert dep_node["properties"]["image"] == "frontend:v2"
        assert dep_node["properties"]["cluster_id"] == "vm-cluster"

    def test_node_capacity_in_properties(self):
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            nodes=[_node("vm2", internal_ip="192.168.56.5")],
        )
        nodes, _ = map_all(inp)
        kn = next(n for n in nodes if n["type"] == "KubernetesNode")
        assert kn["properties"]["internal_ip"] == "192.168.56.5"
        assert kn["properties"]["cpu_capacity"] == "3"
        assert "5440" in kn["properties"]["memory_capacity"]

    def test_pod_health_indicators_in_properties(self):
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-frontend")],
            pods=[_pod("otel-demo-frontend-x", "otel-demo-frontend", ready=False)],
        )
        nodes, _ = map_all(inp)
        pod = next(n for n in nodes if n["type"] == "Pod")
        assert pod["properties"]["ready"] is False
        assert pod["properties"]["phase"] == "Running"
        assert pod["properties"]["host_node"] == "vm2"

    def test_multiple_business_components_share_one_application(self):
        """3 个业务服务挂同一个 Application。"""
        deploys = [_deploy(f"otel-demo-{n}service") for n in ("cart", "payment", "checkout")]
        pods = [
            _pod(f"otel-demo-{n}service-1", f"otel-demo-{n}service")
            for n in ("cart", "payment", "checkout")
        ]
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=deploys,
            pods=pods,
        )
        nodes, edges = map_all(inp)
        nodes = nodes + extract_owner_nodes(nodes)
        apps = [n for n in nodes if n["type"] == "Application"]
        assert len(apps) == 1
        comps = [n for n in nodes if n["type"] == "ApplicationComponent"]
        assert len(comps) == 3
        assert {c["name"] for c in comps} == {"cart", "payment", "checkout"}
        # app -CONTAINS-> 每个 comp
        contains = [e for e in edges
                    if e["source_id"] == apps[0]["id"] and e["relationship_type"] == "CONTAINS"]
        assert len(contains) == 3

    def test_release_prefix_strips_application_name(self):
        """Application name 取 release_prefix。"""
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            release_prefix="otel-demo",
            deployments=[_deploy("otel-demo-frontend")],
            pods=[_pod("otel-demo-frontend-1", "otel-demo-frontend")],
        )
        nodes, _ = map_all(inp)
        nodes = nodes + extract_owner_nodes(nodes)
        app = next(n for n in nodes if n["type"] == "Application")
        assert app["name"] == "otel-demo"

    def test_extract_owner_nodes_idempotent(self):
        """extract_owner_nodes 多次调用不会重复加节点(_extra 已被 pop)。"""
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            deployments=[_deploy("otel-demo-frontend")],
            pods=[_pod("otel-demo-frontend-1", "otel-demo-frontend")],
        )
        nodes, _ = map_all(inp)
        # 第一次抽
        owners1 = extract_owner_nodes(nodes)
        # 第二次抽 — 应该是空(_extra 已 pop)
        owners2 = extract_owner_nodes(nodes)
        assert len(owners1) == 1
        assert len(owners2) == 0

    def test_disconnected_node_still_added(self):
        """KubernetesNode 即使没 Pod 调度上去也入图(集群成员)。"""
        inp = MapperInput(
            cluster_id="vm-cluster",
            namespace="otel-demo",
            nodes=[_node("vm-empty")],
        )
        nodes, _ = map_all(inp)
        ids = {n["id"] for n in nodes}
        assert "node:vm-cluster:vm-empty" in ids

    def test_cluster_id_isolation(self):
        """两个 cluster_id 生成的 ID 互不冲突。"""
        inp_a = MapperInput(cluster_id="cluster-a", namespace="otel-demo",
                            deployments=[_deploy("otel-demo-frontend")],
                            pods=[_pod("otel-demo-frontend-1", "otel-demo-frontend")])
        inp_b = MapperInput(cluster_id="cluster-b", namespace="otel-demo",
                            deployments=[_deploy("otel-demo-frontend")],
                            pods=[_pod("otel-demo-frontend-1", "otel-demo-frontend")])
        nodes_a, _ = map_all(inp_a)
        nodes_b, _ = map_all(inp_b)
        ids_a = {n["id"] for n in nodes_a}
        ids_b = {n["id"] for n in nodes_b}
        assert ids_a.isdisjoint(ids_b)
