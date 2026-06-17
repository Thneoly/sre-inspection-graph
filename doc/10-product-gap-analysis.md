# 10 — 产品对标分析

对比 ChatGPT 对话中定义的 MVP 目标和当前项目实现状态。

## 整体评分

```
MVP 对标: ████████████████░░░░  80%
```

## MVP 输入能力

| 要求 | 状态 | 实现 |
|------|------|------|
| Kubernetes 集群 | ✅ | mock 数据：cluster/namespace/deployment/pod/container |
| Namespace / Application Label | ✅ | Application → Component → Deployment 完整层级 |
| Prometheus | ✅ | MetricQuery (7 种) + MetricSnapshot + PromQL 模板 |
| OpenTelemetry | ❌ | 未实现 |
| 日志系统 | ❌ | 未实现 |
| Git / YAML / Helm | ❌ | 未实现（ConfigMap 基线对比可扩展） |
| 告警规则 | ✅ | AlertRule → AlertEvent，含 severity/status |

## MVP 输出能力

| 要求 | 状态 | 实现 |
|------|------|------|
| 资源对象模型 | ✅ | 30 种节点类型，远超 14 类目标 |
| 实例图谱 | ✅ | L2 实例图含 58 节点 + 161 关系 |
| 巡检图层 | ✅ | 3 层切换：基础拓扑 / 可观测 / 风险巡检 |
| 访问链路图 | ✅ | AccessLinkView: ELB→Ingress→Gateway→Service→Pod |
| 配置风险图 | ✅ | ConfigImpactView: Secret/ConfigMap 影响面 |
| 可观测覆盖图 | ✅ | 可观测图层：MONITORS / VISUALIZES |
| 病变节点清单 | ✅ | health_status (warning/critical) + risk_level |
| 故障传播路径 | ✅ | 级联 + 爆炸半径 + 每类型独立阈值 |
| 快恢动作建议 | ❌ | 未实现 |
| 自检报告 | ❌ | 未实现 |

## 10 个图层（目标 vs 实现）

| # | 图层 | 状态 |
|---|------|------|
| 1 | 基础拓扑 | ✅ 默认层，30 种节点类型 |
| 2 | 业务影响 | ❌ 未实现（需应用级 SLO/评分建模） |
| 3 | 访问链路 | ✅ Ingress → Service → Pod |
| 4 | 可观测 | ✅ 可观测层：AlertRule/Dashboard 覆盖 |
| 5 | 风险巡检 | ✅ 风险层：InspectionFinding/AlertEvent |
| 6 | 告警归并 | ✅ AlertAggregationView: 多告警按应用归并 |
| 7 | 配置变更 | ⚠️ 部分（Secret/ConfigMap 影响视图，无变更事件追踪） |
| 8 | 安全合规 | ❌ |
| 9 | AI 活动 | ❌ |
| 10 | 人机协同 | ❌ |
| 11 | 快恢决策 | ❌ |

## 差异化能力对标

| 能力 | 对话中定义的差异化 | 当前实现 |
|------|-------------------|---------|
| 对象/属性/关系建模 | ✅ | 30 类型 + Neo4j 图 + label 属性模型 |
| 变化事件建模 | ❌ | ChangeEvent 未实现 |
| 病变感知 | ⚠️ | 有 staggered thresholds，无历史基线趋势 |
| 故障推演 | ✅ | 7 种故障 + 级联 + 爆炸半径 + 时间线 |
| 恢复路径 | ❌ | 未实现（InspectionFinding 有 recommendation 字段但未用于恢复编排） |
| 人机协同 | ❌ | 未实现 |
| AI 行为建模 | ❌ | 未实现 |
| 数据孪生 | ⚠️ | DSS 是孪生基础，但未实现完整孪生体同步 |

## 已超额完成的部分

| 项目 | 目标 | 实际 |
|------|------|------|
| 节点类型 | 14 类 | **30 类** |
| 巡检视图 | 4-6 个 | **7 个** |
| 故障类型 | 未定义 | **7 种 + 可扩展** |
| 传播模型 | 未定义 | **blast_radius + cascade + per-type thresholds** |
| 图层 | 未定义 | **3 层可切换** |
| 数据架构 | 直接连 Neo4j | **DSS 解耦 + 内存/Neo4j 双存储** |
| 测试覆盖 | — | **69 tests (53 backend + 16 frontend)** |

## 未覆盖的能力缺口

| 缺口 | 优先级 | 说明 |
|------|--------|------|
| 快恢动作建议 | P0 | InspectionFinding 有 recommendation 文本，需要结构化 + 编排引擎 |
| 自检报告生成 | P1 | 需从巡检结果 + 拓扑 + 指标聚合生成报告 |
| 变化事件 (ChangeEvent) | P1 | 需要追踪配置/部署/资源的变化事件和时间线 |
| 历史基线趋势 | P2 | 需要 MetricSnapshot 历史 + 统计算法检测偏离 |
| 安全合规图层 | P2 | Secret 过期 / CVE 漏洞 / 合规检查 |
| 业务影响图层 | P2 | 应用级 SLO / 健康评分 / 业务指标关联 |
| AI 活动建模 | P3 | Agent / Tool / Prompt / Decision 建模 |
| 人机协同闭环 | P3 | 授权 / 审批 / 回滚 / 反馈 |
| OpenTelemetry 集成 | P3 | Trace 接入，连接 Span → Resource |
| 日志系统集成 | P3 | Loki/ELK 查询入口（已有 log_source 字段） |

## 结论

当前 MVP 完成度约 **80%**。核心差异化能力（图谱建模、故障推演、图层观察、级联传播、DSS 架构）已落地。主要缺口是**快恢动作引擎**和**变化事件追踪**，这是下一步应优先投入的方向。长期愿景中 AI 活动建模和人机协同需要产品定位进一步明确后再启动。
