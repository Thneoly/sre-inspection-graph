"""DSS 内存仓库 — NodeStore / EdgeStore / MetricStore / FaultStore"""
from app.datasource.models import DataNode, DataEdge, MetricSnapshot, FaultInjection


class DataSourceStore:
    """单例内存仓库"""

    def __init__(self):
        self.nodes: dict[str, DataNode] = {}
        self.edges: dict[str, DataEdge] = {}
        self.metrics: dict[str, list[MetricSnapshot]] = {}  # resource_id → [snapshots]
        self.faults: dict[str, FaultInjection] = {}
        self._initialized = False

    # ── Nodes ──
    def get_node(self, node_id: str) -> DataNode | None:
        return self.nodes.get(node_id)

    def get_all_nodes(self) -> list[DataNode]:
        return list(self.nodes.values())

    def upsert_node(self, node: DataNode):
        self.nodes[node.id] = node

    def update_node_props(self, node_id: str, **props):
        if node_id in self.nodes:
            self.nodes[node_id].properties.update(props)

    # ── Edges ──
    def get_edge(self, edge_id: str) -> DataEdge | None:
        return self.edges.get(edge_id)

    def get_all_edges(self) -> list[DataEdge]:
        return list(self.edges.values())

    def upsert_edge(self, edge: DataEdge):
        self.edges[edge.id] = edge

    def update_edge_props(self, edge_id: str, **props):
        if edge_id in self.edges:
            self.edges[edge_id].properties.update(props)

    # ── Metrics ──
    def add_metric(self, snap: MetricSnapshot):
        if snap.resource_id not in self.metrics:
            self.metrics[snap.resource_id] = []
        self.metrics[snap.resource_id].append(snap)

    def get_metrics(self, resource_id: str, n: int = 20) -> list[MetricSnapshot]:
        snaps = self.metrics.get(resource_id, [])
        return sorted(snaps, key=lambda s: s.fetched_at, reverse=True)[:n]

    def clear_fault_metrics(self):
        self.metrics = {k: [s for s in v if not s.snapshot_id.startswith("fault_")] for k, v in self.metrics.items()}

    # ── Faults ──
    def add_fault(self, fault: FaultInjection):
        self.faults[fault.injection_id] = fault

    def get_active_faults(self) -> list[FaultInjection]:
        return [f for f in self.faults.values() if f.status != "resolved"]

    def get_fault(self, fid: str) -> FaultInjection | None:
        return self.faults.get(fid)

    def clear_faults(self):
        self.faults.clear()

    # ── Reset ──
    def reset(self):
        """清除所有运行态数据，保留基线"""
        self.metrics.clear()
        self.faults.clear()


# Global singleton
store = DataSourceStore()
