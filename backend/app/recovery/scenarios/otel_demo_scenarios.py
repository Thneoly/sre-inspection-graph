"""OTel Demo 7 fault scenario 映射 — PRD-004 Sprint 3。

OTel Demo 0.32.0 内置的 feature flag(基于 flagd)可以触发 7 个故障场景。
本模块定义每个 flag → InspectionFinding rule + 推荐 RecoveryAction 的映射,
让平台从"看到 flag 翻转"贯通到"推荐恢复动作"。

每个 scenario 包含:
- flag_name:OTel demo 的 feature flag 名(用于触发/恢复)
- target_component:受影响的 ApplicationComponent short(对应 DSS comp:...)
- expected_metric:开 flag 后哪个 metric 会异常(用于验证 Prom 抓到了)
- finding_rule:对应的 InspectionFinding 规则 ID
- recommended_action:对应的 RecoveryAction(PRD-001 的 8 个之一)
- enable_command / disable_command:操作 flag 的 curl 命令(用于 E2E)

Phase 2:
- 把这些 mapping 接到 InspectionFinding 自动生成器(metric 越线时按 mapping
  生成 finding 并附带 SUGGESTS RecoveryAction 关系)
- 把 enable/disable 也包到 /api/v1/recovery/scenarios/{name}/{enable,disable} 端点
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class FaultScenario:
    """一个 OTel Demo fault scenario 的全部元数据。"""
    name: str
    flag_name: str
    target_component: str          # mapper 短名 (cart, payment, etc)
    expected_metric: str           # span_p99_ms / span_error_rate_pct
    finding_rule: str              # InspectionFinding 规则 ID
    finding_severity: str          # warning | critical
    recommended_action: str        # PRD-001 action_id
    description: str


SCENARIOS: list[FaultScenario] = [
    FaultScenario(
        name="product_catalog_failure",
        flag_name="productCatalogFailure",
        target_component="product-catalog",
        expected_metric="span_error_rate_pct",
        finding_rule="HTTP_5XX_HIGH",
        finding_severity="critical",
        recommended_action="restart_pod",
        description="product-catalog 返回 5xx,推荐重启 Pod 复位状态",
    ),
    FaultScenario(
        name="recommendation_cache_failure",
        flag_name="recommendationServiceCacheFailure",
        target_component="recommendation",
        expected_metric="span_p99_ms",
        finding_rule="MEM_HIGH",
        finding_severity="warning",
        recommended_action="restart_pod",
        description="recommendation 内存泄漏,缓存不停增长。重启清理",
    ),
    FaultScenario(
        name="ad_manual_gc",
        flag_name="adServiceManualGc",
        target_component="ad",
        expected_metric="span_p99_ms",
        finding_rule="P99_HIGH",
        finding_severity="warning",
        recommended_action="rollback_deployment",
        description="ad 手动触发 GC,P99 周期性飙升。回滚到无 GC 版本",
    ),
    FaultScenario(
        name="ad_high_cpu",
        flag_name="adServiceHighCpu",
        target_component="ad",
        expected_metric="span_p99_ms",
        finding_rule="CPU_HIGH",
        finding_severity="critical",
        recommended_action="scale_deployment",
        description="ad CPU 飙满,扩容应对",
    ),
    FaultScenario(
        name="cart_failure",
        flag_name="cartServiceFailure",
        target_component="cart",
        expected_metric="span_error_rate_pct",
        finding_rule="HTTP_5XX_HIGH",
        finding_severity="critical",
        recommended_action="clear_cache",
        description="cart 写入失败,清 Valkey 缓存复位",
    ),
    FaultScenario(
        name="payment_failure",
        flag_name="paymentServiceFailure",
        target_component="payment",
        expected_metric="span_error_rate_pct",
        finding_rule="HTTP_5XX_HIGH",
        finding_severity="critical",
        recommended_action="restart_service",
        description="payment 服务超时,重启 Service 端点恢复路由",
    ),
    FaultScenario(
        name="payment_unreachable",
        flag_name="paymentServiceUnreachable",
        target_component="payment",
        expected_metric="span_error_rate_pct",
        finding_rule="UNREACHABLE",
        finding_severity="critical",
        recommended_action="restart_pod",
        description="payment 完全不可达。重启 Pod 让 Service 重选 endpoint",
    ),
    FaultScenario(
        name="kafka_queue_problems",
        flag_name="kafkaQueueProblems",
        target_component="kafka",
        expected_metric="span_request_rate",
        finding_rule="QUEUE_LAG",
        finding_severity="warning",
        recommended_action="scale_deployment",
        description="kafka lag 飙升。扩容 Deployment",
    ),
]


SCENARIOS_BY_FLAG: dict[str, FaultScenario] = {s.flag_name: s for s in SCENARIOS}
SCENARIOS_BY_NAME: dict[str, FaultScenario] = {s.name: s for s in SCENARIOS}


def scenario_for_flag(flag_name: str) -> FaultScenario | None:
    """flag 名 → scenario 元数据。FlagdConnector 可以查这个表
    在产生 ChangeEvent 时把 scenario.recommended_action 也写进 description。"""
    return SCENARIOS_BY_FLAG.get(flag_name)


def scenario_for_name(name: str) -> FaultScenario | None:
    return SCENARIOS_BY_NAME.get(name)
