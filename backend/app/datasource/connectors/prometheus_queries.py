"""PromQL 查询模板 — 基于 OTel Collector spanmetrics(全 17 服务覆盖)。

OTel Demo 0.32.0 的 Prometheus 通过 OTel Collector 的 spanmetrics 拿到
所有服务的 RED metric(Rate / Errors / Duration),不依赖 cAdvisor / KSM。

关键 metric:
- `duration_milliseconds_bucket / count / sum`
    histogram,带 service_name / span_kind / status_code 标签
- `calls_total`
    所有 span 的累积计数,带相同标签

label `service_name` 直接对应我们的 ApplicationComponent short name
(如 `cartservice` / `frontend` — 与 mapper.normalize_component_name 同源)。

Sprint 2 选三个指标:P99 延迟 / 错误率 / 请求速率。Pod 级 metric 留给 Phase 2。
"""

from __future__ import annotations

from dataclasses import dataclass


# ============================================================
# PromQL 模板
# ============================================================

# P99 延迟 (ms) — span 级 SERVER kind 反映对外服务 SLA
SPAN_P99_LATENCY_MS = """
histogram_quantile(0.99,
  sum by (service_name, le) (
    rate(duration_milliseconds_bucket{span_kind="SPAN_KIND_SERVER"}[5m])
  )
)
"""

# 错误率 (%) — STATUS_CODE_ERROR span 占比
SPAN_ERROR_RATE_PCT = """
100 * sum by (service_name) (
  rate(calls_total{status_code="STATUS_CODE_ERROR", span_kind="SPAN_KIND_SERVER"}[5m])
) /
clamp_min(sum by (service_name) (
  rate(calls_total{span_kind="SPAN_KIND_SERVER"}[5m])
), 0.001)
"""

# 请求速率 (req/s) — span SERVER kind,用于流量基线
SPAN_REQUEST_RATE = """
sum by (service_name) (
  rate(calls_total{span_kind="SPAN_KIND_SERVER"}[5m])
)
"""


# ============================================================
# 查询元数据
# ============================================================

@dataclass(frozen=True)
class QueryDef:
    """一条 PromQL 查询的元数据 — 名字、阈值、目标层级。"""
    name: str
    promql: str
    unit: str
    target: str          # "service" | "pod"
    warning: float
    critical: float
    direction: str = "high"  # high | low — 哪个方向算"差"


# 默认阈值参考 OTel demo 默认负载下的实测水位:p99 服务多在 5-50ms,
# flagd/frontendproxy 偶尔到 2-4 秒(热路径),错误率 <0.1%。
QUERIES: list[QueryDef] = [
    QueryDef(name="span_p99_ms", promql=SPAN_P99_LATENCY_MS,
             unit="ms", target="service", warning=500.0, critical=2000.0),
    QueryDef(name="span_error_rate_pct", promql=SPAN_ERROR_RATE_PCT,
             unit="percent", target="service", warning=1.0, critical=5.0),
    QueryDef(name="span_request_rate", promql=SPAN_REQUEST_RATE,
             unit="req/s", target="service", warning=1e9, critical=1e9,
             direction="high"),  # 没阈值,只采样不告警
]
