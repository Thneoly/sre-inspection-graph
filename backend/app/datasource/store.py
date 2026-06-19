"""DSS 内存仓库 — NodeStore / EdgeStore / MetricStore / FaultStore / RecoveryStore"""
from app.datasource.models import (
    DataNode, DataEdge, MetricSnapshot, FaultInjection,
    RecoveryExecution, ApprovalRequest,
)


class DataSourceStore:
    """单例内存仓库"""

    def __init__(self):
        self.nodes: dict[str, DataNode] = {}
        self.edges: dict[str, DataEdge] = {}
        self.metrics: dict[str, list[MetricSnapshot]] = {}  # resource_id → [snapshots]
        self.faults: dict[str, FaultInjection] = {}
        self.executions: dict[str, RecoveryExecution] = {}      # execution_id → execution
        self.approvals: dict[str, ApprovalRequest] = {}         # approval_id → approval
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

    # ── Recovery Executions ──
    def add_execution(self, execution: RecoveryExecution):
        self.executions[execution.execution_id] = execution

    def get_execution(self, execution_id: str) -> RecoveryExecution | None:
        return self.executions.get(execution_id)

    def get_all_executions(self) -> list[RecoveryExecution]:
        return list(self.executions.values())

    def update_execution(self, execution: RecoveryExecution):
        """覆盖式更新(execution.status 变化时调用)。"""
        self.executions[execution.execution_id] = execution

    def clear_executions(self):
        self.executions.clear()

    # ── Approval Requests ──
    def add_approval(self, approval: ApprovalRequest):
        self.approvals[approval.approval_id] = approval

    def get_approval(self, approval_id: str) -> ApprovalRequest | None:
        return self.approvals.get(approval_id)

    def get_pending_approvals(self) -> list[ApprovalRequest]:
        return [a for a in self.approvals.values() if a.approval_status == "pending"]

    def update_approval(self, approval: ApprovalRequest):
        """覆盖式更新审批状态。"""
        self.approvals[approval.approval_id] = approval

    def get_approvals_by_status(self, status: str | None = None) -> list[ApprovalRequest]:
        """按 status 过滤;status=None 返回全部。"""
        if status is None:
            return list(self.approvals.values())
        return [a for a in self.approvals.values() if a.approval_status == status]

    def clear_approvals(self):
        self.approvals.clear()

    # ── Reset ──
    def reset(self):
        """清除所有运行态数据，保留基线"""
        self.metrics.clear()
        self.faults.clear()
        # executions / approvals 不清——历史是审计资产


# Global singleton
store = DataSourceStore()
