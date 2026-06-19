"""Trace → CALLS 边聚合 — 纯函数,易测试。

Jaeger trace 结构:
  trace
    ├─ spans[]  ← 每个 span 有 process(serviceName) + references(parent span)
    └─ processes{}  ← span.processID → {serviceName, tags}

我们把 span A.process_service → span B.process_service 当作 caller→callee
(B 是 A 的 child span,通过 references[].refType=CHILD_OF 关联)。

聚合规则:
- 5 分钟窗口内 (caller, callee) 出现次数 >= threshold → 创建 CALLS 边
- 阈值默认 5(在 settings.jaeger_call_count_threshold)
- 阈值低 → 边多噪声;阈值高 → 漏掉低频依赖。5 是实测平衡点
"""

from __future__ import annotations

import logging
from collections import Counter
from typing import Any, Iterable

from app.datasource.connectors.k8s_mapper import (
    component_id, normalize_component_name,
)


logger = logging.getLogger(__name__)


def aggregate_calls_from_traces(
    traces: Iterable[dict],
    cluster_id: str,
    namespace: str,
    release_prefix: str = "otel-demo",
    threshold: int = 5,
) -> tuple[Counter, list[dict]]:
    """聚合 trace 数据 → CALLS 边列表。

    返回 (counter, edges):
        counter — 原始 (caller, callee) → count(用于调试 / 单元测试断言)
        edges   — 满足阈值的 CALLS DataEdge dict 列表(供 connector 写 DSS)
    """
    counter: Counter = Counter()

    for trace in traces:
        spans = trace.get("spans", []) or []
        processes = trace.get("processes", {}) or {}
        # span_id → service_name
        span_to_service: dict[str, str] = {}
        for span in spans:
            pid = span.get("processID", "")
            sname = (processes.get(pid, {}) or {}).get("serviceName", "")
            sid = span.get("spanID", "")
            if sid and sname:
                span_to_service[sid] = sname

        # 遍历 child span,看父 span 的 service → 形成 caller→callee
        for span in spans:
            child_service = span_to_service.get(span.get("spanID", ""))
            if not child_service:
                continue
            for ref in span.get("references", []) or []:
                if ref.get("refType") != "CHILD_OF":
                    continue
                parent_id = ref.get("spanID", "")
                parent_service = span_to_service.get(parent_id)
                if not parent_service:
                    continue
                if parent_service == child_service:
                    continue  # 同服务内调用不算
                counter[(parent_service, child_service)] += 1

    # 转成 DataEdge dict
    edges: list[dict] = []
    for (caller, callee), count in counter.items():
        if count < threshold:
            continue
        caller_id = _service_to_component_id(caller, cluster_id, namespace, release_prefix)
        callee_id = _service_to_component_id(callee, cluster_id, namespace, release_prefix)
        if not caller_id or not callee_id:
            continue
        edges.append({
            "id": f"{caller_id}|CALLS|{callee_id}",
            "source_id": caller_id,
            "target_id": callee_id,
            "relationship_type": "CALLS",
            "relationship_name": "调用",
            "properties": {
                "dependency_strength": "中",
                "discovery_method": "jaeger_connector",
                "call_count_5m": count,
            },
        })

    return counter, edges


def _service_to_component_id(
    service_name: str,
    cluster_id: str,
    namespace: str,
    release_prefix: str = "otel-demo",
) -> str:
    """OTel service.name → DSS component ID。

    OTel demo 服务的 service.name 跟 deployment short name 一致(如 `cartservice`),
    跟 mapper.normalize_component_name 同源。
    """
    if not service_name:
        return ""
    comp_short = normalize_component_name(f"{release_prefix}-{service_name}", release_prefix)
    return component_id(cluster_id, namespace, comp_short)
