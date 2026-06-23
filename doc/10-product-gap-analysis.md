# 10 — 产品对标分析

对比 ChatGPT 对话中定义的 MVP 目标和当前项目实现状态。本文档随实施进度滚动更新。

## 整体评分

```
MVP 对标:    ████████████████████  100%  (PRD-001/002/003/004 全部落地)
v2 规划:    ████░░░░░░░░░░░░░░░░   20%  (PRD-005/006 已立项,未实施)
```

## MVP 输入能力

| 要求 | 状态 | 实现 |
|------|------|------|
| Kubernetes 集群 | ✅ | K8sConnector(PRD-004)真实接 OTel demo + 离线 CSV 兜底 |
| Namespace / Application Label | ✅ | Application → Component → Deployment 完整层级 |
| Prometheus | ✅ | PromQL 3 模板(p99/error/qps)+ MetricSnapshot + AlertEvent + AlertRule |
| OpenTelemetry | ✅ | JaegerConnector(CALLS 边),trace span attrs 增强规划在 PRD-005 S2 |
| 日志系统 | ❌ | 仍未实现,延后 |
| Git / YAML / Helm | ⚠️ | 部分 — ArgoCD/Harbor webhook 已接(PRD-002),完整代码仓接入在 PRD-006 |
| 告警规则 | ✅ | AlertRule → AlertEvent,含 severity/status,connector 自动产 |

## MVP 输出能力

| 要求 | 状态 | 实现 |
|------|------|------|
| 资源对象模型 | ✅ | 30+ 种节点类型 |
| 实例图谱 | ✅ | L2 实例图 + 真实 K8s 同步 |
| 巡检图层 | ✅ | 3 层切换:基础拓扑 / 可观测 / 风险巡检 |
| 访问链路图 | ✅ | AccessLinkView: ELB→Ingress→Gateway→Service→Pod |
| 配置风险图 | ✅ | ConfigImpactView + Secret/ConfigMap ChangeEvent 时间线 |
| 可观测覆盖图 | ✅ | 可观测图层:MONITORS / VISUALIZES |
| 病变节点清单 | ✅ | health_status + risk_level + connector 自动推导 |
| 故障传播路径 | ✅ | 级联 + 爆炸半径 + 每类型独立阈值 |
| 快恢动作建议 | ✅ | PRD-001 — 8 actions + 跨集群编排 + 自动验证 + 动作链 |
| 自检报告 | ✅ | PRD-003 — 3 模板 + 12 模块 + 邮件订阅 + APScheduler |

## 11 个图层(目标 vs 实现)

| # | 图层 | 状态 |
|---|------|------|
| 1 | 基础拓扑 | ✅ 默认层,30+ 节点类型 |
| 2 | 业务影响 | ⚠️ Health Score 已实现(PRD-003),无 SLO 评分 |
| 3 | 访问链路 | ✅ ELB → Ingress → Service → Pod |
| 4 | 可观测 | ✅ AlertRule/Dashboard 覆盖 |
| 5 | 风险巡检 | ✅ InspectionFinding / AlertEvent |
| 6 | 告警归并 | ✅ AlertAggregationView |
| 7 | 配置变更 | ✅ PRD-002 ChangeEvent + 时间线 + Git/CI 关联 |
| 8 | 安全合规 | ❌ 延后(漏洞继承等) |
| 9 | AI 活动 | ❌ 延后 |
| 10 | 人机协同 | ✅ PRD-001 审批中心 + 24h TTL + 回滚 + 动作链 |
| 11 | 快恢决策 | ✅ PRD-001 完整动作引擎 |

## 差异化能力对标

| 能力 | 对话中定义的差异化 | 当前实现 |
|------|-------------------|---------|
| 对象/属性/关系建模 | ✅ | 30+ 类型 + Neo4j 图 + label 属性模型 + DSS 孪生 |
| 变化事件建模 | ✅ | PRD-002 ChangeEvent + correlated query + propagation BFS + CORRELATED_WITH 边 |
| 病变感知 | ✅ | staggered thresholds + Prometheus connector 自动推导 health + AlertRule 阈值告警 |
| 故障推演 | ✅ | 7 种故障 + 级联 + 爆炸半径 + 时间线 |
| 恢复路径 | ✅ | PRD-001 — 8 actions + dry-run + 审批流 + 跨集群 + 自动验证 + 动作链 |
| 人机协同 | ✅ | PRD-001 审批中心 + PRD-003 邮件订阅 |
| AI 行为建模 | ❌ | 未实现 |
| 数据孪生 | ✅ | DSS 内存图作为孪生体,connector 持续同步 |

## 已超额完成的部分

| 项目 | 目标 | 实际 |
|------|------|------|
| 节点类型 | 14 类 | **30+ 类** |
| 巡检视图 | 4-6 个 | **12 个**(7 巡检 + 4 恢复 + 报告 + Connector) |
| 故障类型 | 未定义 | **7 种 + 可扩展** |
| 传播模型 | 未定义 | **blast_radius + cascade + per-type thresholds** |
| 图层 | 未定义 | **3 层可切换** |
| 数据架构 | 直接连 Neo4j | **DSS 解耦 + 6 connector + Neo4j dual-write** |
| 测试覆盖 | — | **543 tests (472 backend + 71 frontend)** |
| 跨集群恢复 | 未定义 | **PRD-001 Phase 2 — k8s_client switch-and-reload + cluster_id 路由** |
| 动作链编排 | 未定义 | **CHAIN_TEMPLATES + 3 失败策略 + 链级单次审批** |

## 当前已知缺口(链到规划 PRD)

| 缺口 | 优先级 | 规划归属 |
|------|--------|---------|
| **集群外资产盲区**(ELB / APIG / 托管 MySQL/Redis/Kafka) | P0 | **PRD-005 S4** Cloud API connector |
| **客户端 SDK 嵌入依赖**(trace 看到但图里没的外部 SaaS) | P0 | **PRD-005 S2** trace_aggregator 升级 + **S3** Unknown Dep Queue |
| **多源数据合并机制**(K8s vs Cloud API vs Trace 同一资源去重) | P0 | **PRD-005 S1** Fact 总线 + Identity Resolver |
| **代码仓接入**(PR/MR 事件 / 业务规则抽取 / repo 元数据) | P1 | **PRD-006** |
| **服务注册中心**(Nacos / Consul) | P1 | **PRD-005 S5** Config plane connector |
| **网关控制面**(Kong / APISIX) | P1 | **PRD-005 S5** Gateway connector |
| **GitOps intent drift**(declared vs actual) | P1 | **PRD-005 S6** ArgoCD CR + Terraform tfstate |
| K8s Ingress / PVC / HPA 同步 | P2 | PRD-005 S1(K8sConnector 扩展) |
| 历史基线趋势 + 偏离检测 | P2 | 后续 PRD,需 MetricSnapshot 历史归档 |
| 安全合规图层(CVE / Secret 过期) | P2 | 后续 PRD,可继承 PRD-006 Library 节点 |
| 业务影响图层(SLO 评分) | P3 | 后续 PRD,可从 PRD-006 抽出的 SLO 注解派生 |
| AI 活动建模 | P3 | 产品定位明确后启动 |
| 日志系统集成(Loki/ELK) | P3 | 后续 PRD,已有 log_source 字段占位 |
| eBPF / Network flow | P4 | PRD-005 不做 |

## 演进路径总览

```
v0 静态 CSV
    ↓ (PRD-001/002/003/004 已完成)
v1 单源 connector + 完整恢复/变更/报告闭环  ← 我们在这里
    ↓ (PRD-005 + PRD-006)
v2 统一拓扑感知:
   - Fact 总线 + Identity Resolver 横切层
   - Trace-driven Unknown Dependency Queue
   - Cloud API / Gateway / Config plane / GitOps / 代码仓 多源接入
   - 业务规则从代码自动抽取
    ↓ (Phase 4+)
v3 高阶能力:
   - 安全合规图层(漏洞继承)
   - 业务 SLO 评分
   - 日志系统集成
   - AI 活动建模(可选)
```

## 结论

**MVP 阶段已 100% 完成**:四层模型、DSS 孪生、6 connector、PRD-001/002/003/004 全套闭环、543 测试覆盖。

**当前最大缺口是数据广度**(集群外资产、外部 SaaS、代码仓上游),不是数据深度。**PRD-005 是底座重构**(把"N 个 connector 各写各的"改成"统一 Fact 总线");**PRD-006 是数据源新增**(代码仓 + 业务规则抽取)。两者解耦设计,可并行开发,但 PRD-006 依赖 PRD-005 的 Fact 总线就绪。

下一步建议从 **PRD-005 Sprint 2(trace_aggregator 增强,单文件改动)** 入手 — ROI 最高,能立刻补半张图。
