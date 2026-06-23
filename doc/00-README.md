# 00 — 文档导航

本目录共 13 份文档(含本文件),按"读者动机"分四组。建议新人从对应分组的第一篇看起。

## 📁 分组速查

### A. 入门 / 全局视图(必读)

| # | 文档 | 一句话 | 何时读 |
|---|------|--------|--------|
| 01 | [需求概述与平台目标](./01-requirements-overview.md) | 平台定位、四层模型、技术栈 | **第一篇** |
| 10 | [产品对标分析](./10-product-gap-analysis.md) | MVP 完成度 100% / v2 规划 20%,缺口分布 | 评估当前能力 |
| 13 | [Unknown Dep 端到端剧本](./13-story-unknown-dep-stripe.md) | Stripe trace → Queue → 代码仓 → 节点 | 想直观理解 UTS 怎么工作 |

### B. 数据模型(L1-L4 静态规约,实现稳定)

| # | 文档 | 一句话 |
|---|------|--------|
| 02 | [L1/L2 类型与实例模型](./02-L1-L2-type-and-instance-model.md) | 14 类型节点 + 35 关系 |
| 03 | [L3 动态观测模型](./03-L3-dynamic-metrics-model.md) | MetricQuery / Snapshot / AlertEvent |
| 04 | [L4 巡检结果模型](./04-L4-inspection-results-model.md) | InspectionRun / Rule / Finding |
| 08 | [故障类型与时间线](./08-fault-types-and-timeline.md) | 7 种故障 + 多级级联 + 每类型阈值 |
| 09 | [数据源服务 DSS](./09-data-source-service.md) | 内存孪生层,connector 写入入口 |

### C. 实现交付物(已落地的 PRD)

| 范围 | 文档锚点 | 状态 |
|------|---------|------|
| **PRD-001** 恢复动作引擎 | CLAUDE.md / README.md(本目录暂无独立 PRD 文档,以代码 + 工作文档为准) | ✅ 100% |
| **PRD-002** 变更事件追踪 | 同上 | ✅ 100% |
| **PRD-003** 自检报告 | 同上 | ✅ 100% |
| **PRD-004** OTel Demo 真实接入 | 同上 | ✅ 100% |
| 视图 / API / 前端 | [05](./05-six-views-design.md) / [06](./06-api-specification.md) / [07](./07-frontend-component-tree.md) | ✅ |

> **说明**:PRD-001/002/003/004 因迭代节奏快(每个 PRD 跨多个 Sprint + Phase 2 补丁),设计描述维护在 `CLAUDE.md` 的对应章节里,与代码同步。`doc/` 下的 PRD 文档只在**实施前的规划阶段**写入(见 D 组)。

### D. 规划中的 PRD(未实施,详细设计已就绪)

| # | 文档 | 一句话 | 优先级 |
|---|------|--------|--------|
| 11 | [PRD-005 统一拓扑服务 UTS](./11-PRD-005-universal-topology-service.md) | Fact 总线 + Identity Resolver 横切层,N→1 统一合并 | **下一步** |
| 12 | [PRD-006 代码仓数据源](./12-PRD-006-code-repo-source.md) | 业务规则 / PR-MR 事件 / 构建链路自动抽取,依赖 PRD-005 | 紧随 PRD-005 |

## 🧭 按角色选路

### 你是新加入的 SRE / 平台工程师
**01 → 02 → 09 → 05 → 13** — 看懂平台是什么、数据怎么建模、视图能查什么,最后用一个完整故事串起来。

### 你是来评估"还能做什么"
**01 → 10 → 11 → 12** — 看现状,看差距,看下一步两个 PRD 解决什么。

### 你是即将上手实施 PRD-005 / PRD-006 的开发
**11 / 12 全读** → **13 剧本对照** → CLAUDE.md 找 BaseConnector / DSS 现有模式 → 直接照 Sprint Plan 写代码。
PRD-005 推荐入手点是 **Sprint 2(trace_aggregator 升级,单文件)** — ROI 最高、影响面小、能立刻补半张图。

### 你只想看视图能展示什么
**05 → 07** — 6 视图 + 4 个 PRD 视图(审批中心 / 恢复历史 / 恢复链 / 报告中心 / 变更时间线 / Connector 状态),前端组件树。

## 🔗 PRD 间依赖

```
PRD-001 ──┐
PRD-002 ──┤── 已完成(MVP) ──┐
PRD-003 ──┤                 │
PRD-004 ──┘                 │
                            ▼
                    ┌── PRD-005 (UTS 底座)
                    │     │
                    │     ▼
                    └── PRD-006 (代码仓,消费 Fact 总线)
                          │
                          ▼
                  v3 高阶:安全合规图层 / SLO 评分 / AI 活动建模
```

PRD-005 是 PRD-006 的硬前置 — 代码仓 connector 通过 Fact 总线注入,而不是直接写 DSS。

## 📜 演进时间线

```
2026 Q1   PRD-001/002/003/004 全部上线(MVP 100% 完成)
2026 Q2   ▶ 当前
  ↓
2026 Q3   PRD-005 S1+S2 落地:Fact 总线 + trace_aggregator 升级
          → trace 看得到的客户端嵌入依赖进图
2026 Q4   PRD-005 S3-S6:Unknown Dep Queue / Cloud API / Gateway / GitOps
2027 Q1   PRD-006 S1+S2 落地:代码仓接入 + 业务规则抽取
```

## 📝 文档约定

- 每份 PRD 文档统一 **14 节**:背景 / 设计原则 / 目标架构 / 核心契约 / 关键算法 / 工业对标 / Sprint 计划 / 验收 / 风险 / 不做 / File Map(其余按需)
- L1-L4 模型文档稳定,改动需在 CHANGELOG 留痕
- 已实施 PRD 的"在做"细节走 `CLAUDE.md`,**不**塞进 `doc/`(避免实施波动污染规划文档)
- 跨文档链接用相对路径 `./11-PRD-005-...md`,不用绝对 URL

## 🔍 找东西

```bash
# 找 PRD-005 / PRD-006 相关
grep -l "PRD-005\|PRD-006" doc/*.md

# 找某个数据模型术语(如 TopologyFact)
grep -rn "TopologyFact" doc/

# 找某个节点类型(如 CodeRepo)
grep -rn "CodeRepo" doc/
```
