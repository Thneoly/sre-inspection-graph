"""应用配置 — 从环境变量加载"""
import os
from dataclasses import dataclass, field


def _parse_kubeconfigs() -> dict[str, str]:
    """支持 KUBECONFIGS='vm-cluster=~/.kube/vm,kind-local=~/.kube/kind' 形式。"""
    raw = os.getenv("KUBECONFIGS", "")
    if not raw:
        return {}
    out: dict[str, str] = {}
    for pair in raw.split(","):
        if "=" not in pair:
            continue
        name, path = pair.split("=", 1)
        out[name.strip()] = os.path.expanduser(path.strip())
    return out


@dataclass
class Settings:
    neo4j_uri: str = os.getenv("NEO4J_URI", "bolt://localhost:7687")
    neo4j_user: str = os.getenv("NEO4J_USER", "neo4j")
    neo4j_password: str = os.getenv("NEO4J_PASSWORD", "sre-inspection")
    neo4j_max_connection_lifetime: int = 3600
    neo4j_max_connection_pool_size: int = 50
    neo4j_connection_acquisition_timeout: int = 10

    # ============================================================
    # PRD-004 — Connector configs(K8s / Prometheus / Jaeger / flagd)
    # ============================================================
    # 集群名 → kubeconfig 路径。空 dict 表示 connector 关闭。
    kubeconfigs: dict[str, str] = field(default_factory=_parse_kubeconfigs)
    active_cluster: str = os.getenv("ACTIVE_CLUSTER", "vm-cluster")
    k8s_namespace: str = os.getenv("K8S_NAMESPACE", "otel-demo")
    k8s_sync_interval_seconds: int = int(os.getenv("K8S_SYNC_INTERVAL", "30"))

    # 启动时是否自动启动 connectors(测试场景设为 0)
    connectors_autostart: bool = os.getenv("CONNECTORS_AUTOSTART", "1") == "1"


settings = Settings()
