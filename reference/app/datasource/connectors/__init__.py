"""数据源 connector 包 — PRD-004。

每个 connector 周期性从外部源(K8s API / Prometheus / Jaeger / flagd)拉数据,
diff 后写入 DSS。所有 connector 共用 BaseConnector 抽象。

包结构:
- base.py            — BaseConnector 抽象类 + SyncResult dataclass
- k8s_connector.py   — K8s 拓扑同步(Sprint 1)
- k8s_mapper.py      — K8s 对象 → DataNode/DataEdge(Sprint 1,纯函数)
- sync_orchestrator.py — startup 拉起所有 connector

Sprint 2 加 prometheus_connector.py / jaeger_connector.py / trace_aggregator.py。
Sprint 3 加 flagd_connector.py / k8s_event_connector.py。
"""
