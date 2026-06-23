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

    # PRD-002 Phase 2 — K8s watch connector(长连接实时监听 CM/Secret/Deployment)
    # 默认 0:watch 长连接不适合测试/CI;vm 集群才开
    k8s_watch_enabled: bool = os.getenv("K8S_WATCH_ENABLED", "0") == "1"

    # PRD-002 Phase 2 — webhook 接收 token(空则跳过校验,PoC 简化;生产必开)
    webhook_token: str = os.getenv("WEBHOOK_TOKEN", "")

    # ─── Prometheus ───
    prometheus_url: str = os.getenv("PROMETHEUS_URL", "http://localhost:19090")
    prometheus_sync_interval_seconds: int = int(os.getenv("PROMETHEUS_SYNC_INTERVAL", "30"))

    # ─── Jaeger ───
    # OTel Demo Helm chart 给 Jaeger 配的 base-path 是 /jaeger/ui
    # API 路径变成 /jaeger/ui/api/services 而不是 /api/services
    jaeger_url: str = os.getenv("JAEGER_URL", "http://localhost:16686/jaeger/ui")
    jaeger_sync_interval_seconds: int = int(os.getenv("JAEGER_SYNC_INTERVAL", "300"))
    jaeger_lookback_seconds: int = int(os.getenv("JAEGER_LOOKBACK", "300"))
    jaeger_call_count_threshold: int = int(os.getenv("JAEGER_CALL_COUNT_THRESHOLD", "5"))

    # ─── flagd ───
    flagd_url: str = os.getenv("FLAGD_URL", "http://localhost:8013")
    flagd_sync_interval_seconds: int = int(os.getenv("FLAGD_SYNC_INTERVAL", "20"))

    # ============================================================
    # PRD-001 Phase 2 — 真实 handler 开关 + MySQL/Redis 连接
    # ============================================================
    # mock = 仅改 DSS 孪生(默认,测试安全);real = 调真实 K8s/MySQL/Redis API
    recovery_handler_mode: str = os.getenv("RECOVERY_HANDLER_MODE", "mock")

    # ─── MySQL(kill_query)─── 优先用 DSS 节点 properties 里的连接信息,缺失走全局默认
    mysql_host: str = os.getenv("MYSQL_HOST", "")
    mysql_port: int = int(os.getenv("MYSQL_PORT", "3306"))
    mysql_user: str = os.getenv("MYSQL_USER", "")
    mysql_password: str = os.getenv("MYSQL_PASSWORD", "")
    mysql_database: str = os.getenv("MYSQL_DATABASE", "")

    # ─── Redis(clear_cache)───
    redis_host: str = os.getenv("REDIS_HOST", "")
    redis_port: int = int(os.getenv("REDIS_PORT", "6379"))
    redis_password: str = os.getenv("REDIS_PASSWORD", "")
    redis_db: int = int(os.getenv("REDIS_DB", "0"))


settings = Settings()
