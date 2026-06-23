# 13 — 端到端剧本:从 Stripe 调用看 UTS 怎么工作

一个完整故事,串联 **PRD-005(统一拓扑服务)** + **PRD-006(代码仓数据源)** + 现有 **PRD-002(变更追踪)** 三块,让"为什么需要这套设计"立刻可感。

> 这不是规范文档,是**导览**。所有节点、ID、字段都按当前规划稿对齐;读完知道每一步发生在哪份 PRD 的哪一节。

---

## 0. 场景设定

**时间**:2026-07-15 14:32

**业务**:订单服务 `payment-service` 在 OTel Demo 集群里跑,5 分钟前刚发布了 `v2.3.0`。

**症状**:checkout 接口 P99 从 80ms 飙到 2.4s。SRE 打开告警归并视图,看到 `payment-service` 标红、`AlertEvent: p99_breach_critical` firing。

**预期**:30 秒内,SRE 看到"图上原本没有的一个外部依赖 Stripe API",一键确认 → 节点入图 → 后续变更/告警自动联动。

---

## 1. 当前架构(无 UTS,无 PRD-006)看到的画面

SRE 点开 `payment-service` 节点,右侧 NodeDetailPanel 看到:

```
payment-service (Deployment)
├── BELONGS_TO → comp:vm-cluster:otel-demo:payment
├── DEPLOYED_AS → 3 × pod:vm-cluster:otel-demo:payment-xxx
├── CALLS → cart-service  (来自 Jaeger 聚合,call_count_5m=42)
└── (空)
```

**问题**:Stripe 是 payment 的核心下游,trace 里能看到 `https://api.stripe.com/v1/charges` 的 HTTP span,但因为 Stripe 不在 K8s 集群、没在 `scripts/add_infra_nodes.py` 里手工建过节点,**它在图上根本不存在**。

- 告警归并不会把"Stripe 报错"算进 payment 影响面
- 变更时间线看不到"Stripe API 版本升级"
- 恢复动作引擎不知道有 Stripe 这个依赖

这就是 PRD-005 §2 列的 **"集群外资产盲区"** 痛点。

---

## 2. 启用 PRD-005 S2:trace_aggregator 升级(挖 OTel span attrs)

PRD-005 Sprint 2 的产出物 — 升级现有 `jaeger_connector.py`,从 span attrs 里挖三类客户端嵌入依赖:

- `db.system` = mysql / postgresql / redis → 数据库依赖
- `messaging.system` = kafka / rabbitmq → 消息依赖  
- `peer.service` / `server.address` / `http.url` host → **外部 HTTP 依赖**

### 14:32:05 — trace_aggregator 30s sync 跑了一次

抓到 5 分钟内的 trace,发现:

```
trace_id: abc123...
└── span: POST /charges
    attrs:
      http.method = POST
      http.url = https://api.stripe.com/v1/charges
      http.status_code = 200
      peer.service = api.stripe.com
      server.address = api.stripe.com
    parent: payment-service
```

trace_aggregator 不再只产 `CALLS` 边,而是发 **TopologyFact**(PRD-005 §4):

```python
TopologyFact(
    source="jaeger",
    observed_at=2026-07-15T14:32:05Z,
    ttl_seconds=600,
    confidence=0.6,                    # trace 推断,中等置信度
    fact_type="node",
    payload={
        "node_type": "ExternalService",
        "display_name": "api.stripe.com",
        "endpoint": "https://api.stripe.com",
        "observed_via": "trace_http_call",
    },
    correlation_keys=[
        "domain:api.stripe.com",        # OTel SemConv 对齐
        "endpoint:https://api.stripe.com",
    ],
)

TopologyFact(  # 同时产边
    source="jaeger",
    fact_type="edge",
    payload={
        "edge_type": "CALLS",
        "src_id": "deploy:vm-cluster:otel-demo:payment-service",
        "dst_correlation_key": "domain:api.stripe.com",   # 关键:用 key 而不是 id
        "call_count_5m": 217,
        "error_count_5m": 0,
    },
    correlation_keys=["domain:api.stripe.com"],
)
```

### 14:32:06 — Fact 总线 → Identity Resolver

PRD-005 §5 的 Identity Resolver 收到这两条 Fact:

1. **查现有图**:`correlation_keys=["domain:api.stripe.com"]` 在 DSS 索引里**找不到匹配**
2. **判定**:这是个"图里没有的新节点"
3. **行为**:不直接建节点(避免噪音爆炸),而是**塞进 Unknown Dependency Queue**(PRD-005 §6)

```python
UnknownDependency(
    queue_id="unk:domain:api.stripe.com:2026-07-15T14:32:05Z",
    correlation_keys=["domain:api.stripe.com"],
    observed_facts=[fact1, fact2],
    observed_from=["deploy:vm-cluster:otel-demo:payment-service"],
    first_seen=2026-07-15T14:32:05Z,
    last_seen=2026-07-15T14:32:05Z,
    sample_count=1,
    suggested_node_type="ExternalService",
)
```

---

## 3. PRD-006 介入:用代码仓富化

PRD-006 §3 设计的 code_repo_connector 在 14:30 那次同步里(每小时一跑),已经把 `payment-service` 仓库(`gitlab.example.com/team-pay/payment-service`)的依赖清单和源码索引了进来。

### 14:32:07 — Unknown Dep Queue 触发 enrichment

PRD-005 §6 + PRD-006 §6 的联动:Queue 新增条目时,自动调 code_repo enrichment hook,查代码仓里有没有 `api.stripe.com` 字面量:

```bash
# 服务侧只是这个等价 grep
grep -rn "api.stripe.com" repos/team-pay/payment-service/
# →
src/payment/stripe_client.py:14:STRIPE_API_BASE = "https://api.stripe.com"
src/payment/stripe_client.py:42:    response = requests.post(f"{STRIPE_API_BASE}/v1/charges", ...)
requirements.txt:8:stripe==7.12.0
```

enrichment 给 UnknownDependency 加了 **3 条上下文证据**:

```python
queue_entry.code_evidence = [
    {
        "repo_url": "gitlab.example.com/team-pay/payment-service",
        "file": "src/payment/stripe_client.py",
        "line": 14,
        "commit_sha": "a3f9c1d",
        "type": "url_literal",
    },
    {
        "repo_url": "...",
        "file": "requirements.txt",
        "type": "dependency_manifest",
        "library": "pkg:pypi/stripe@7.12.0",   # purl
    },
    {
        "repo_url": "...",
        "file": "src/payment/stripe_client.py",
        "line": 42,
        "type": "outbound_http_call",
        "extracted_rule": "stripe_charge_call",  # PRD-006 §7 业务规则抽取
    },
]
queue_entry.confidence_boost = 0.3        # 0.6 (trace) + 0.3 (code) = 0.9
queue_entry.suggested_owner_team = "team-pay"  # 从代码仓 group 推断
```

现在这条 Unknown Dep **不再是"trace 看到一个陌生域名"**,而是:

> **payment-service 的 `stripe_client.py:14` 在调 Stripe API,使用 `pkg:pypi/stripe@7.12.0`,业主 team-pay。置信度 0.9。**

---

## 4. SRE 视图:Unknown Dep Queue 出现

前端会有一个新页面 `/unknown-dependencies`(PRD-005 §7 列在 S3 交付物里)。SRE 看到:

```
┌────────────────────────────────────────────────────────────────────┐
│ ⚠ 1 条未确认依赖                                                    │
├────────────────────────────────────────────────────────────────────┤
│ api.stripe.com (HTTPS)                                  [置信 0.9] │
│   来源:trace(217 次/5m,0 错)                                    │
│   被 deploy:vm-cluster:otel-demo:payment-service 调用               │
│   代码证据:                                                        │
│     • stripe_client.py:14 (url_literal, commit a3f9c1d)            │
│     • requirements.txt: stripe==7.12.0                              │
│   建议:ExternalService 节点 | 业主 team-pay                       │
│                                                                    │
│   [一键入图]  [标记忽略]  [合并到现有节点...]                       │
└────────────────────────────────────────────────────────────────────┘
```

SRE 点 **[一键入图]**。

---

## 5. 14:33:10 — 节点入图,关系自动建好

后端做 4 件事:

1. **建节点**:
   ```cypher
   MERGE (:ExternalService:ResourceInstance {
       node_id: "ext:api.stripe.com",
       display_name: "api.stripe.com",
       endpoint: "https://api.stripe.com",
       owner_team: "team-pay",
       discovery_method: "unknown_dep_promoted",
       correlation_keys: ["domain:api.stripe.com", "endpoint:https://api.stripe.com"],
   })
   ```

2. **建边**:trace 之前缓存的 CALLS Fact 现在可以兑现 — `dst_correlation_key` 被 Identity Resolver 翻译成新节点 id,写入 DSS + Neo4j。

3. **PRD-006 顺带写入**(因为 enrichment 已经知道代码仓和库):
   ```cypher
   MERGE (:CodeRepo {node_id: "repo:gitlab:team-pay:payment-service"})
   MERGE (:Library {node_id: "pkg:pypi/stripe@7.12.0"})
   MERGE (repo)-[:DEFINES]->(ext)
   MERGE (repo)-[:DEPENDS_ON]->(lib)
   ```

4. **Queue 出队**:`unknown_dep.status = "promoted"`,关联 `promoted_node_id` 留审计。

---

## 6. 回到故障现场:30 秒后的改观

SRE 切回告警归并视图。`payment-service` 还红着,但现在多了一行:

```
payment-service ─CALLS→ api.stripe.com
                          └─ stripe 7.12.0 (pypi)
                          └─ owner: team-pay
                          └─ p99 / error_rate 未接入(灰色,提示去 Cloud API connector 配 Stripe 监控)
```

更关键的是 — **变更时间线**(PRD-002)看到 14:30 那次 `payment-service` rollout 时,自动把"`pkg:pypi/stripe@6.4.0 → 7.12.0`"(从 requirements.txt diff 提取)作为风险标签挂在 ChangeEvent 上(PRD-006 §4 ChangeEvent 扩展)。

**故障定位结论**:Stripe SDK 大版本升级导致 HTTP 调用 timeout 默认值变化,P99 飙升源于此。SRE 在恢复动作中心点 `rollback_deployment payment-service` → 流量回 v2.2 → 恢复。

---

## 7. 这个剧本里发生了什么

| 步骤 | 发生在 | 对应 PRD 章节 |
|------|--------|---------------|
| trace 挖 span attrs 产 Fact | jaeger_connector 升级版 | **PRD-005 §3 + S2** |
| Fact 总线 + Identity Resolver 不匹配 → Queue | dispatcher + resolver | **PRD-005 §4 + §5** |
| Unknown Dep Queue + 代码仓 enrichment | enrichment hook | **PRD-005 §6 + PRD-006 §6** |
| 一键入图 + 关系兑现 | promote API + 缓存 fact 应用 | **PRD-005 §6 末** |
| CodeRepo / Library / DEFINES / DEPENDS_ON | code_repo_connector 先期写入 | **PRD-006 §3 + §5** |
| ChangeEvent + 风险标签 | requirements.txt diff → 业务规则 | **PRD-006 §4 + §7** |
| 故障定位 + 恢复 | 现有视图自动联动 | 已实现 PRD-001/002 |

**没有 PRD-005 / PRD-006 的话**:Stripe 永远不在图里;SRE 要花 15 分钟看 trace + 翻代码 + 问 team-pay 才知道"是 Stripe SDK 升级"。

**有了之后**:30 秒,图自己告诉你。

---

## 8. 给实施者的最小可验证切片

如果想最快摸到这条链路的"半张图",**第一周做这三件事就行**(对应 PRD-005 Sprint 2):

1. 改 `backend/app/datasource/connectors/trace_aggregator.py`,从 span 里多读 `peer.service` / `server.address`(8-10 行新增)
2. 写到 DSS 时如果目标 component_id 不存在 → 暂时建一个 `:ExternalService` 节点,标 `discovery_method=trace_inferred`(20 行 mapper 升级)
3. 前端 NodeDetailPanel 加一段"客户端嵌入依赖"(complaint:这一步可以最后做,后端先跑起来)

这只是 PRD-005 Sprint 2 的小一半,但**已经能在 OTel Demo 上看到 5-10 个之前看不到的外部依赖**(`flagd.example.com`, `*.googleapis.com` 之类)。是验证"统一拓扑感知"思路的最低成本切入。

剩下的 Fact 总线 / Identity Resolver / Unknown Dep Queue / 代码仓接入都是水到渠成的扩展 — 但**没有这一步,后面所有横切层设计都是 PPT**。

---

## 9. 相关文档

- 整体动机和当前缺口 — [10-product-gap-analysis.md](./10-product-gap-analysis.md)
- UTS 完整设计 — [11-PRD-005-universal-topology-service.md](./11-PRD-005-universal-topology-service.md)
- 代码仓接入完整设计 — [12-PRD-006-code-repo-source.md](./12-PRD-006-code-repo-source.md)
- 现有数据流(L1-L4) — [01](./01-requirements-overview.md) / [09](./09-data-source-service.md)
