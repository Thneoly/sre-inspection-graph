"""JaegerConnector — Jaeger HTTP API → CALLS 边聚合。

工作流(每 5 分钟):
1. GET /api/services 拿全 namespace 服务名
2. 对每个 service GET /api/traces?service=X&lookback=5m&limit=N 拉最近 trace
3. 调 trace_aggregator.aggregate_calls_from_traces 得到 CALLS 边
4. 写 DSS:diff + upsert + 删除消失的 jaeger_connector 边

设计要点:
- 5 分钟窗口 + count>=5 阈值过滤噪声
- 删除:只删 discovery_method=jaeger_connector 的边,不动其他来源
- limit_per_service 默认 100,够样本但不打爆 Jaeger
"""

from __future__ import annotations

import logging
from typing import Any, Optional

import httpx

from app.config import settings
from app.datasource.connectors.base import BaseConnector, SyncResult
from app.datasource.connectors.trace_aggregator import aggregate_calls_from_traces
from app.datasource.models import DataEdge
from app.datasource.store import store


logger = logging.getLogger(__name__)


class JaegerConnector(BaseConnector):
    name = "jaeger"

    def __init__(
        self,
        jaeger_url: Optional[str] = None,
        cluster_id: Optional[str] = None,
        namespace: Optional[str] = None,
        sync_interval_seconds: Optional[int] = None,
        lookback_seconds: Optional[int] = None,
        threshold: Optional[int] = None,
        release_prefix: str = "otel-demo",
        limit_per_service: int = 100,
        timeout_seconds: float = 10.0,
    ):
        super().__init__()
        self.jaeger_url = (jaeger_url or settings.jaeger_url).rstrip("/")
        self.cluster_id = cluster_id or settings.active_cluster
        self.namespace = namespace or settings.k8s_namespace
        self.sync_interval_seconds = sync_interval_seconds or settings.jaeger_sync_interval_seconds
        self.lookback_seconds = lookback_seconds or settings.jaeger_lookback_seconds
        self.threshold = threshold if threshold is not None else settings.jaeger_call_count_threshold
        self.release_prefix = release_prefix
        self.limit_per_service = limit_per_service
        self.timeout_seconds = timeout_seconds

    async def sync_once(self) -> SyncResult:
        result = SyncResult()
        if not self.jaeger_url:
            result.notes.append("jaeger_url is empty, skipping")
            return result

        all_traces: list[dict] = []
        async with httpx.AsyncClient(timeout=self.timeout_seconds) as client:
            try:
                services = await self._list_services(client)
            except Exception as e:  # noqa: BLE001
                result.notes.append(f"list services failed: {e}")
                return result

            for svc in services:
                if not self._is_otel_demo_service(svc):
                    continue
                try:
                    traces = await self._list_traces(client, svc)
                except Exception as e:  # noqa: BLE001
                    result.notes.append(f"traces({svc}) failed: {e}")
                    continue
                all_traces.extend(traces)

        counter, edge_dicts = aggregate_calls_from_traces(
            all_traces,
            cluster_id=self.cluster_id,
            namespace=self.namespace,
            release_prefix=self.release_prefix,
            threshold=self.threshold,
        )

        # 写 DSS — diff
        new_edge_ids: set[str] = {e["id"] for e in edge_dicts}
        existing_edge_ids = {
            e.id for e in store.get_all_edges()
            if (e.properties or {}).get("discovery_method") == "jaeger_connector"
        }

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

        # 删消失的
        for eid in existing_edge_ids - new_edge_ids:
            if eid in store.edges:
                del store.edges[eid]
                result.edges_removed += 1

        result.notes.append(
            f"traces={len(all_traces)} pairs={len(counter)} "
            f"above_threshold={len(edge_dicts)} threshold={self.threshold}"
        )
        return result

    # ============================================================
    # Jaeger HTTP API
    # ============================================================

    async def _list_services(self, client: httpx.AsyncClient) -> list[str]:
        resp = await client.get(f"{self.jaeger_url}/api/services")
        resp.raise_for_status()
        body = resp.json()
        return body.get("data", []) or []

    async def _list_traces(self, client: httpx.AsyncClient, service: str) -> list[dict]:
        # Jaeger 'lookback' 参数是绝对时间 us;用 'start'/'end' 更靠谱
        # 简化:用 'lookback' 字符串(支持 5m/1h)更直观
        params = {
            "service": service,
            "lookback": f"{self.lookback_seconds}s",
            "limit": str(self.limit_per_service),
        }
        resp = await client.get(f"{self.jaeger_url}/api/traces", params=params)
        resp.raise_for_status()
        body = resp.json()
        return body.get("data", []) or []

    def _is_otel_demo_service(self, service_name: str) -> bool:
        """过滤掉 jaeger 自身 / load-generator / etc。"""
        skip = {"jaeger-all-in-one", "jaeger-query", "jaeger-collector",
                "loadgenerator", "load-generator", "load_generator"}
        return service_name not in skip
