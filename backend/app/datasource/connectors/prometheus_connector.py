"""PrometheusConnector — Prom HTTP API → DSS MetricSnapshot + 节点 health 推导。

工作流:
1. 启动:从 settings.prometheus_url 拿地址(默认 http://localhost:19090)
2. sync_once 每 30 秒:
   - 跑 5 条 PromQL 查询(POD CPU/mem/restart + APP 5xx/P99)
   - 把 sample 反查 DSS 节点 ID(label.pod / label.service_name)
   - 写 MetricSnapshot 到 DSS,并更新 node.properties.health = green/yellow/red
   - 返回 SyncResult(metrics_added)

设计要点:
- 反查 Pod ID:标签里有 namespace + pod,我们的 DSS Pod ID 模式是
    `pod:{cluster}:{namespace}:{pod_name}`
- 反查 Service / Component ID:OTel 的 `service_name` label = 我们的 component name
    (比如 `cartservice` → comp short name `cart`),需要做反向命名映射
    但 OTel demo 的 service_name 实际是 chart 给的 deployment short(如 `cartservice`)
    这里跟 mapper.normalize_component_name 保持一致

不在本类:
- PromQL 模板(在 prometheus_queries.py)
- 健康度规则(在 health_rules.py)
"""

from __future__ import annotations

import logging
import os
import uuid
from datetime import datetime, timezone
from typing import Any, Optional

import httpx

from app.config import settings
from app.datasource.connectors.base import BaseConnector, SyncResult
from app.datasource.connectors.health_rules import derive_health, evaluate_breach
from app.datasource.connectors.k8s_mapper import (
    component_id, normalize_component_name, pod_id,
)
from app.datasource.connectors.prometheus_queries import QUERIES
from app.datasource.models import MetricSnapshot
from app.datasource.store import store


logger = logging.getLogger(__name__)


class PrometheusConnector(BaseConnector):
    """Prometheus → DSS metric + health。"""

    name = "prometheus"

    def __init__(
        self,
        prometheus_url: Optional[str] = None,
        cluster_id: Optional[str] = None,
        namespace: Optional[str] = None,
        sync_interval_seconds: Optional[int] = None,
        release_prefix: str = "otel-demo",
        timeout_seconds: float = 5.0,
    ):
        super().__init__()
        self.prometheus_url = (prometheus_url or settings.prometheus_url).rstrip("/")
        self.cluster_id = cluster_id or settings.active_cluster
        self.namespace = namespace or settings.k8s_namespace
        self.sync_interval_seconds = sync_interval_seconds or settings.prometheus_sync_interval_seconds
        self.release_prefix = release_prefix
        self.timeout_seconds = timeout_seconds

    async def sync_once(self) -> SyncResult:
        result = SyncResult()
        if not self.prometheus_url:
            result.notes.append("prometheus_url is empty, skipping")
            return result

        new_snapshots: list[MetricSnapshot] = []
        affected_node_ids: set[str] = set()

        async with httpx.AsyncClient(timeout=self.timeout_seconds) as client:
            for q in QUERIES:
                try:
                    samples = await self._query(client, q.promql)
                except Exception as e:  # noqa: BLE001
                    result.notes.append(f"query {q.name} failed: {e}")
                    continue
                for labels, value in samples:
                    target_id = self._resolve_target_id(q.target, labels)
                    if not target_id:
                        continue
                    snap = self._make_snapshot(target_id, q.name, value, q.unit)
                    new_snapshots.append(snap)
                    affected_node_ids.add(target_id)

        # 写入 DSS
        for snap in new_snapshots:
            store.add_metric(snap)
            result.metrics_added += 1

        # 推导 health 并更新节点
        for nid in affected_node_ids:
            recent = store.get_metrics(nid, n=len(QUERIES) * 2)
            health = derive_health(recent)
            if health is None:
                continue
            node = store.get_node(nid)
            if node is None:
                continue
            old_health = (node.properties or {}).get("health")
            if old_health != health:
                node.properties["health"] = health
                result.nodes_updated += 1

        result.notes.append(
            f"queries={len(QUERIES)} samples={result.metrics_added} affected={len(affected_node_ids)}"
        )
        return result

    # ============================================================
    # 内部
    # ============================================================

    async def _query(self, client: httpx.AsyncClient, promql: str) -> list[tuple[dict, float]]:
        """跑一条 PromQL,返回 [(labels, value), ...]。"""
        resp = await client.get(
            f"{self.prometheus_url}/api/v1/query",
            params={"query": promql.strip()},
        )
        resp.raise_for_status()
        data = resp.json()
        if data.get("status") != "success":
            raise RuntimeError(f"prom query failed: {data.get('error', 'unknown')}")
        result = data.get("data", {}).get("result", [])
        out: list[tuple[dict, float]] = []
        for entry in result:
            metric_labels = entry.get("metric", {})
            try:
                value = float(entry.get("value", [0, "0"])[1])
            except (TypeError, ValueError, IndexError):
                continue
            if value != value:  # NaN
                continue
            out.append((metric_labels, value))
        return out

    def _resolve_target_id(self, target: str, labels: dict) -> str:
        """label → DSS 节点 ID。

        - target=service:OTel spanmetrics 的 `service_name` 标签直接对应我们
            ApplicationComponent short name(如 cartservice / frontend)。
        - target=pod:本 Sprint 没接 cAdvisor,这条路径 dead;留给 Phase 2。
        """
        if target == "service":
            svc = labels.get("service_name", "")
            if not svc:
                return ""
            comp = normalize_component_name(f"{self.release_prefix}-{svc}", self.release_prefix)
            return component_id(self.cluster_id, self.namespace, comp)
        if target == "pod":
            pod_name = labels.get("pod", "")
            ns = labels.get("namespace", self.namespace)
            if not pod_name:
                return ""
            return pod_id(self.cluster_id, ns, pod_name)
        return ""

    def _make_snapshot(self, target_id: str, metric_name: str, value: float, unit: str) -> MetricSnapshot:
        warn, crit = evaluate_breach(metric_name, value)
        return MetricSnapshot(
            snapshot_id=f"prom-{uuid.uuid4().hex[:12]}",
            resource_id=target_id,
            metric_name=metric_name,
            current_value=float(value),
            unit=unit,
            fetched_at=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            warning_breached=warn,
            critical_breached=crit,
        )
