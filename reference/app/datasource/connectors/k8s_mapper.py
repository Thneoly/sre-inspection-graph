"""K8s 对象 → DataNode/DataEdge 纯函数 mapper。

设计原则:
- **不**调用任何 K8s API,只接 dataclass / dict 形态的 K8s 对象做转换。
- 这样测试零成本(直接构造 dict),也方便 Phase 2 替换为 informer 缓存。
- 命名遵循 PRD-002 既有 convention:
    `{type}:{cluster}:{namespace}:{name}` 形式的 ID。
- 关系边遵循 view4_config_impact 同款规则:
    app -CONTAINS-> comp -DEPLOYED_AS-> deploy -CONTAINS-> pod
    pod -USES-> configmap/secret
    pod -SCHEDULED_ON-> node
    svc -ROUTES_TO-> pod
    pod -DEPENDS_ON-> redis/kafka/postgres(本 Sprint 不推导,等 Sprint 2 trace 接入)

不入图(infrastructure 不当 ApplicationComponent):
- loadgenerator / otelcol / prometheus-server / jaeger
- (它们仍会作为 Pod/Deployment 节点入图,只是不挂在 Application 下)

中间件名字识别:
- valkey  → Redis
- kafka   → Kafka
- postgres / postgresql → PostgreSQL(PRD-004 新增类型)
- mysql   → MySQL
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Optional


# ============================================================
# 常量
# ============================================================

# OTel Demo Helm release 默认前缀。从 deployment 名字 strip 出 component 名。
DEFAULT_RELEASE_PREFIX = "otel-demo"

# 不当 ApplicationComponent 的 infra deploy(只入图为 Deployment/Pod 节点)。
INFRA_NAMES: set[str] = {
    "loadgenerator",
    "otelcol",
    "prometheus-server",
    "jaeger",
    "opensearch",
    "grafana",
    "kibana",
}

# 中间件名字识别 → (本平台节点 type, ID 前缀)。
MIDDLEWARE_PATTERNS: dict[str, tuple[str, str]] = {
    "valkey": ("Redis", "redis"),
    "redis": ("Redis", "redis"),
    "kafka": ("Kafka", "kafka"),
    "postgres": ("PostgreSQL", "postgres"),
    "postgresql": ("PostgreSQL", "postgres"),
    "mysql": ("MySQL", "mysql"),
}


# ============================================================
# 输入轻量 dataclass — 测试方便构造,不依赖 kubernetes-asyncio
# ============================================================

@dataclass
class K8sObjectRef:
    """简化的 K8s 对象引用 — namespace/kind/name + labels + 必要字段。"""
    name: str
    namespace: str
    kind: str
    labels: dict[str, str]
    annotations: dict[str, str]
    spec: dict[str, Any]
    status: dict[str, Any]
    uid: str = ""

    def label(self, key: str, default: str = "") -> str:
        return self.labels.get(key, default)


@dataclass
class MapperInput:
    """一次同步的全部 K8s 对象 — 测试构造 + sync 时填充。"""
    cluster_id: str
    namespace: str
    release_prefix: str = DEFAULT_RELEASE_PREFIX
    nodes: list[K8sObjectRef] = None  # type: ignore[assignment]
    deployments: list[K8sObjectRef] = None  # type: ignore[assignment]
    pods: list[K8sObjectRef] = None  # type: ignore[assignment]
    services: list[K8sObjectRef] = None  # type: ignore[assignment]
    configmaps: list[K8sObjectRef] = None  # type: ignore[assignment]
    secrets: list[K8sObjectRef] = None  # type: ignore[assignment]

    def __post_init__(self):
        for f in ("nodes", "deployments", "pods", "services", "configmaps", "secrets"):
            if getattr(self, f) is None:
                setattr(self, f, [])


# ============================================================
# ID 生成器(public — 测试也用)
# ============================================================

def app_id(cluster: str, namespace: str, release: str = DEFAULT_RELEASE_PREFIX) -> str:
    return f"app:{cluster}:{namespace}:{release}"


def component_id(cluster: str, namespace: str, comp_name: str) -> str:
    return f"comp:{cluster}:{namespace}:{comp_name}"


def deployment_id(cluster: str, namespace: str, name: str) -> str:
    return f"deploy:{cluster}:{namespace}:{name}"


def pod_id(cluster: str, namespace: str, name: str) -> str:
    return f"pod:{cluster}:{namespace}:{name}"


def service_id(cluster: str, namespace: str, name: str) -> str:
    return f"svc:{cluster}:{namespace}:{name}"


def configmap_id(cluster: str, namespace: str, name: str) -> str:
    return f"configmap:{cluster}:{namespace}:{name}"


def secret_id(cluster: str, namespace: str, name: str) -> str:
    return f"secret:{cluster}:{namespace}:{name}"


def k8s_node_id(cluster: str, name: str) -> str:
    return f"node:{cluster}:{name}"


def middleware_id(cluster: str, namespace: str, name: str, type_prefix: str) -> str:
    return f"{type_prefix}:{cluster}:{namespace}:{name}"


# ============================================================
# 命名规则
# ============================================================

def normalize_component_name(deploy_name: str, release_prefix: str = DEFAULT_RELEASE_PREFIX) -> str:
    """从 deployment 名字推 ApplicationComponent 短名。

    例:
        otel-demo-cartservice → cart
        otel-demo-frontendproxy → frontend-proxy
        otel-demo-frontend → frontend
        otel-demo-recommendationservice → recommendation
        otel-demo-frauddetectionservice → fraud-detection
        otel-demo-productcatalogservice → product-catalog
    """
    name = deploy_name
    if name.startswith(release_prefix + "-"):
        name = name[len(release_prefix) + 1:]
    # 砍 "service" 后缀(只在长度>10 时,避免吃掉短名 e.g. "ad" 没问题)
    if name.endswith("service") and len(name) > len("service"):
        name = name[: -len("service")]
    # 已知混淆名拆分
    name = (
        name
        .replace("frauddetection", "fraud-detection")
        .replace("productcatalog", "product-catalog")
        .replace("frontendproxy", "frontend-proxy")
    )
    return name


def detect_middleware(deploy_name: str, release_prefix: str = DEFAULT_RELEASE_PREFIX) -> Optional[tuple[str, str]]:
    """看 deployment 名字像不像中间件 → 返回 (节点 type, ID 前缀)。

    匹配规则:strip release prefix 后,看尾段在 MIDDLEWARE_PATTERNS 里。
    例:otel-demo-valkey → ("Redis", "redis")
    """
    short = deploy_name
    if short.startswith(release_prefix + "-"):
        short = short[len(release_prefix) + 1:]
    # 直接整名匹配
    if short in MIDDLEWARE_PATTERNS:
        return MIDDLEWARE_PATTERNS[short]
    # 兜底:看 keyword 子串(valkey-cart / kafka-broker 这种)
    for kw, mapping in MIDDLEWARE_PATTERNS.items():
        if kw in short:
            return mapping
    return None


def is_infra(deploy_name: str, release_prefix: str = DEFAULT_RELEASE_PREFIX) -> bool:
    """是不是 infrastructure(不当 ApplicationComponent)。"""
    short = deploy_name
    if short.startswith(release_prefix + "-"):
        short = short[len(release_prefix) + 1:]
    if short in INFRA_NAMES:
        return True
    # prometheus-server 这种带 - 后缀的
    for infra in INFRA_NAMES:
        if short.startswith(infra):
            return True
    return False


# ============================================================
# 主 mapper
# ============================================================

def map_all(inp: MapperInput) -> tuple[list[dict], list[dict]]:
    """主入口:全部 K8s 对象 → ([node_dict], [edge_dict])。

    返回 dict 而非 DataNode/DataEdge,让上层(connector)决定是否 wrap;
    单元测试也更方便比对。
    """
    nodes: list[dict] = []
    edges: list[dict] = []

    # 1. Application(synthetic,一个 release 一个)
    nodes.append(_make_application(inp))

    # 2. KubernetesNode
    for kn in inp.nodes:
        nodes.append(_make_k8s_node(kn, inp.cluster_id))

    # 3. Deployment / Component / 中间件
    deploy_to_pods: dict[str, list[K8sObjectRef]] = _group_pods_by_owner(inp.pods)
    referenced_cms: set[str] = set()
    referenced_secrets: set[str] = set()

    for dep in inp.deployments:
        dep_node, dep_edges, _owner_id = _make_deployment_with_owner(dep, inp)
        nodes.append(dep_node)
        edges.extend(dep_edges)

        # Pods under deployment
        for pod in deploy_to_pods.get(dep.name, []):
            pod_node, pod_edges, cms, secs = _make_pod(pod, dep, inp)
            nodes.append(pod_node)
            edges.extend(pod_edges)
            referenced_cms.update(cms)
            referenced_secrets.update(secs)

    # 4. Service
    pod_index: dict[str, K8sObjectRef] = {p.name: p for p in inp.pods}
    for svc in inp.services:
        svc_node, svc_edges = _make_service(svc, pod_index, inp)
        nodes.append(svc_node)
        edges.extend(svc_edges)

    # 5. ConfigMap / Secret(只入图 Pod 实际挂载的)
    for cm in inp.configmaps:
        if cm.name in referenced_cms:
            nodes.append(_make_configmap(cm, inp))
    for sec in inp.secrets:
        if sec.name in referenced_secrets:
            nodes.append(_make_secret(sec, inp))

    return nodes, edges


# ============================================================
# 内部 helper
# ============================================================

def _make_application(inp: MapperInput) -> dict:
    aid = app_id(inp.cluster_id, inp.namespace, inp.release_prefix)
    return {
        "id": aid,
        "type": "Application",
        "name": inp.release_prefix,
        "properties": {
            "node_id": aid,
            "cluster_id": inp.cluster_id,
            "namespace": inp.namespace,
            "release": inp.release_prefix,
            "owner_team": "platform",
            "discovery_method": "k8s_connector",
        },
    }


def _make_k8s_node(kn: K8sObjectRef, cluster_id: str) -> dict:
    nid = k8s_node_id(cluster_id, kn.name)
    capacity = (kn.status or {}).get("capacity", {}) or {}
    addresses = (kn.status or {}).get("addresses", []) or []
    internal_ip = next((a.get("address", "") for a in addresses if a.get("type") == "InternalIP"), "")
    return {
        "id": nid,
        "type": "KubernetesNode",
        "name": kn.name,
        "properties": {
            "node_id": nid,
            "cluster_id": cluster_id,
            "internal_ip": internal_ip,
            "cpu_capacity": str(capacity.get("cpu", "")),
            "memory_capacity": str(capacity.get("memory", "")),
            "pod_capacity": str(capacity.get("pods", "")),
            "discovery_method": "k8s_connector",
        },
    }


def _make_deployment_with_owner(
    dep: K8sObjectRef, inp: MapperInput,
) -> tuple[dict, list[dict], str]:
    """生成 Deployment 节点 + 与 owner(Component / 中间件)的关系边。

    返回 (deployment_node, edges, owner_id)。
    """
    dep_id = deployment_id(inp.cluster_id, inp.namespace, dep.name)
    spec = dep.spec or {}
    status = dep.status or {}

    # Image — 取第一个 container
    containers = (spec.get("template", {}).get("spec", {}) or {}).get("containers", []) or []
    primary_image = containers[0].get("image", "") if containers else ""

    dep_node = {
        "id": dep_id,
        "type": "Deployment",
        "name": dep.name,
        "properties": {
            "node_id": dep_id,
            "cluster_id": inp.cluster_id,
            "namespace": inp.namespace,
            "image": primary_image,
            "replicas_desired": spec.get("replicas", 0),
            "replicas_ready": status.get("readyReplicas", 0),
            "discovery_method": "k8s_connector",
        },
    }

    edges: list[dict] = []
    owner_id: str = ""

    if is_infra(dep.name, inp.release_prefix):
        # infra:不挂 application,owner_id 留空
        return dep_node, edges, ""

    middleware = detect_middleware(dep.name, inp.release_prefix)
    if middleware:
        mw_type, mw_prefix = middleware
        owner_id = middleware_id(inp.cluster_id, inp.namespace, dep.name, mw_prefix)
        edges.append(_edge(owner_id, "DEPLOYED_AS", "部署为", dep_id, "强"))
        # 中间件节点本身
        # 反向构造一个"middleware node"塞进 nodes_extra 是不优雅的 —
        # 改成把它通过返回值携带出去,在 map_all 里加。这里用 trick:
        # 将"待加节点"挂到 dep_node 的私有 _extra_nodes,map_all 不读它。
        # 不,clean approach:直接 append 到 edges 列表的"前面"是 ok,
        # 但我们没法在这里 append node。所以重构:
        # —— 直接在 map_all 里处理中间件节点。
        # 这里只产生 owner_id 让上层加节点。
        # 实际:map_all 里看到 owner_id 是中间件就加 node。
        # 简化做法:把中间件节点放进 dep_node 的 properties._extra
        dep_node["properties"]["_middleware_owner"] = {
            "id": owner_id, "type": mw_type, "name": dep.name,
            "cluster_id": inp.cluster_id, "namespace": inp.namespace,
        }
        return dep_node, edges, owner_id

    # 普通业务服务 → ApplicationComponent
    comp_name = normalize_component_name(dep.name, inp.release_prefix)
    comp_id_v = component_id(inp.cluster_id, inp.namespace, comp_name)
    owner_id = comp_id_v
    aid = app_id(inp.cluster_id, inp.namespace, inp.release_prefix)
    edges.append(_edge(aid, "CONTAINS", "包含", comp_id_v, "强"))
    edges.append(_edge(comp_id_v, "DEPLOYED_AS", "部署为", dep_id, "强"))
    dep_node["properties"]["_component_owner"] = {
        "id": comp_id_v, "type": "ApplicationComponent", "name": comp_name,
        "cluster_id": inp.cluster_id, "namespace": inp.namespace,
        "deployment_name": dep.name,
    }
    return dep_node, edges, owner_id


def _make_pod(
    pod: K8sObjectRef, owning_deploy: K8sObjectRef, inp: MapperInput,
) -> tuple[dict, list[dict], set[str], set[str]]:
    pid = pod_id(inp.cluster_id, inp.namespace, pod.name)
    dep_id = deployment_id(inp.cluster_id, inp.namespace, owning_deploy.name)
    spec = pod.spec or {}
    status = pod.status or {}
    node_name = spec.get("nodeName", "")
    pod_phase = status.get("phase", "")

    pod_node = {
        "id": pid,
        "type": "Pod",
        "name": pod.name,
        "properties": {
            "node_id": pid,
            "cluster_id": inp.cluster_id,
            "namespace": inp.namespace,
            "phase": pod_phase,
            "pod_ip": status.get("podIP", ""),
            "host_node": node_name,
            "ready": _pod_is_ready(status),
            "restart_count": _sum_restart_count(status),
            "discovery_method": "k8s_connector",
        },
    }

    edges: list[dict] = [
        _edge(dep_id, "CONTAINS", "包含", pid, "强"),
    ]
    if node_name:
        nid = k8s_node_id(inp.cluster_id, node_name)
        edges.append(_edge(pid, "SCHEDULED_ON", "调度在", nid, "强"))

    cm_refs: set[str] = set()
    secret_refs: set[str] = set()

    # volumes → CM/Secret
    for vol in spec.get("volumes", []) or []:
        if "configMap" in vol and vol["configMap"]:
            cm_name = vol["configMap"].get("name", "")
            if cm_name:
                cm_refs.add(cm_name)
                edges.append(_edge(pid, "USES", "使用",
                                   configmap_id(inp.cluster_id, inp.namespace, cm_name), "强"))
        if "secret" in vol and vol["secret"]:
            sec_name = vol["secret"].get("secretName", "")
            if sec_name:
                secret_refs.add(sec_name)
                edges.append(_edge(pid, "USES", "使用",
                                   secret_id(inp.cluster_id, inp.namespace, sec_name), "强"))

    # envFrom → CM/Secret
    for c in spec.get("containers", []) or []:
        for ef in c.get("envFrom", []) or []:
            if "configMapRef" in ef and ef["configMapRef"]:
                cm_name = ef["configMapRef"].get("name", "")
                if cm_name:
                    cm_refs.add(cm_name)
                    edges.append(_edge(pid, "USES", "使用",
                                       configmap_id(inp.cluster_id, inp.namespace, cm_name), "强"))
            if "secretRef" in ef and ef["secretRef"]:
                sec_name = ef["secretRef"].get("name", "")
                if sec_name:
                    secret_refs.add(sec_name)
                    edges.append(_edge(pid, "USES", "使用",
                                       secret_id(inp.cluster_id, inp.namespace, sec_name), "强"))

    return pod_node, edges, cm_refs, secret_refs


def _make_service(
    svc: K8sObjectRef, pod_index: dict[str, K8sObjectRef], inp: MapperInput,
) -> tuple[dict, list[dict]]:
    sid = service_id(inp.cluster_id, inp.namespace, svc.name)
    spec = svc.spec or {}
    selector = spec.get("selector", {}) or {}
    ports = spec.get("ports", []) or []

    svc_node = {
        "id": sid,
        "type": "Service",
        "name": svc.name,
        "properties": {
            "node_id": sid,
            "cluster_id": inp.cluster_id,
            "namespace": inp.namespace,
            "cluster_ip": spec.get("clusterIP", ""),
            "service_type": spec.get("type", "ClusterIP"),
            "ports": ",".join(str(p.get("port", "")) for p in ports),
            "discovery_method": "k8s_connector",
        },
    }

    edges: list[dict] = []
    if selector:
        # selector 匹配的 pod
        for pod in pod_index.values():
            if _labels_match(pod.labels, selector):
                edges.append(_edge(sid, "ROUTES_TO", "路由到",
                                   pod_id(inp.cluster_id, inp.namespace, pod.name), "强"))
    return svc_node, edges


def _make_configmap(cm: K8sObjectRef, inp: MapperInput) -> dict:
    cid = configmap_id(inp.cluster_id, inp.namespace, cm.name)
    return {
        "id": cid,
        "type": "ConfigMap",
        "name": cm.name,
        "properties": {
            "node_id": cid,
            "cluster_id": inp.cluster_id,
            "namespace": inp.namespace,
            "data_keys": ",".join(sorted((cm.spec or {}).get("data", {}).keys())),
            "discovery_method": "k8s_connector",
        },
    }


def _make_secret(sec: K8sObjectRef, inp: MapperInput) -> dict:
    sid = secret_id(inp.cluster_id, inp.namespace, sec.name)
    return {
        "id": sid,
        "type": "Secret",
        "name": sec.name,
        "properties": {
            "node_id": sid,
            "cluster_id": inp.cluster_id,
            "namespace": inp.namespace,
            "secret_type": (sec.spec or {}).get("type", "Opaque"),
            "discovery_method": "k8s_connector",
        },
    }


# ============================================================
# 工具
# ============================================================

def _edge(source_id: str, rel_type: str, rel_name: str, target_id: str, strength: str) -> dict:
    return {
        "id": f"{source_id}|{rel_type}|{target_id}",
        "source_id": source_id,
        "target_id": target_id,
        "relationship_type": rel_type,
        "relationship_name": rel_name,
        "properties": {
            "dependency_strength": strength,
            "discovery_method": "k8s_connector",
        },
    }


def _group_pods_by_owner(pods: Iterable[K8sObjectRef]) -> dict[str, list[K8sObjectRef]]:
    """按 owner deployment 名字归组。owner 推导:看 ownerReferences.replicaset → 抠 deployment 前缀。

    K8s 实际链路是 Pod → ReplicaSet → Deployment,
    我们的 K8sObjectRef 简化没有 ReplicaSet 中间层,
    所以 spec / status 里如果带 owner_deployment 提示就用,没有就靠 pod label。
    """
    out: dict[str, list[K8sObjectRef]] = {}
    for pod in pods:
        owner = _resolve_pod_owner_deployment(pod)
        if owner:
            out.setdefault(owner, []).append(pod)
    return out


def _resolve_pod_owner_deployment(pod: K8sObjectRef) -> str:
    """从 pod 推 owner Deployment 名字。

    优先级:
    1. pod.spec['_owner_deployment'](测试构造 / connector 预填)
    2. labels['app.kubernetes.io/component'] + release_prefix(otel demo 风格)
    3. labels['app']
    """
    # connector 预填的 owner(从 ReplicaSet ownerReferences 反查)
    direct = (pod.spec or {}).get("_owner_deployment", "")
    if direct:
        return direct
    # OTel demo helm 标签:每个 pod 有 app.kubernetes.io/name=<deployment>
    for key in ("app.kubernetes.io/name", "app.kubernetes.io/component", "app"):
        if key in pod.labels:
            return pod.labels[key]
    return ""


def _labels_match(pod_labels: dict[str, str], selector: dict[str, str]) -> bool:
    return all(pod_labels.get(k) == v for k, v in selector.items())


def _pod_is_ready(status: dict[str, Any]) -> bool:
    for cond in status.get("conditions", []) or []:
        if cond.get("type") == "Ready" and cond.get("status") == "True":
            return True
    return False


def _sum_restart_count(status: dict[str, Any]) -> int:
    total = 0
    for cs in status.get("containerStatuses", []) or []:
        total += int(cs.get("restartCount", 0))
    return total


# ============================================================
# 把 _middleware_owner / _component_owner 合成节点 — 给 connector 调用
# ============================================================

def extract_owner_nodes(deployment_nodes: list[dict]) -> list[dict]:
    """从 deployment node 的 properties._component_owner / _middleware_owner 抽出 owner 节点。

    这样设计的原因:owner 推导逻辑在 _make_deployment_with_owner 里,不便回头扫一遍 deployments;
    把临时载体塞进 dep_node 然后这里捞出来,clean 且不重复推导。
    """
    seen: set[str] = set()
    out: list[dict] = []
    for dep_node in deployment_nodes:
        props = dep_node.get("properties", {})
        for key in ("_component_owner", "_middleware_owner"):
            owner = props.pop(key, None)
            if owner and owner["id"] not in seen:
                seen.add(owner["id"])
                out.append({
                    "id": owner["id"],
                    "type": owner["type"],
                    "name": owner["name"],
                    "properties": {
                        "node_id": owner["id"],
                        "cluster_id": owner["cluster_id"],
                        "namespace": owner["namespace"],
                        "discovery_method": "k8s_connector",
                        **{k: v for k, v in owner.items() if k not in ("id", "type", "name", "cluster_id", "namespace")},
                    },
                })
    return out
