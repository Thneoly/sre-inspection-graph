# 11 — PRD-005 统一拓扑感知服务 (Universal Topology Service)

> **状态**:设计阶段(待评审)
> **依赖**:PRD-004(connector 框架已就绪)、PRD-002(ChangeEvent + 关联机制已就绪)
> **影响范围**:数据层底座重构 — 内存孪生层 升级 + 新增 3 个子模块 + 既有 6 个 connector 改造

---

## 1. 背景与问题陈述

PRD-004 完成后,平台具备 6 个 connector(K8s / Prometheus / Jaeger / flagd / K8s-events / K8s-watch)持续同步真实集群拓扑到 内存孪生层。但**现网部署的资源远不止 K8s 集群内**:

- **集群外网络设施**:ELB / APIG / Gateway / CDN / WAF
- **集群外数据底座**:托管 RDS / 云 Redis / 云 Kafka / 对象存储 / ES 集群
- **服务注册中心**:Nacos / Consul / Eureka
- **声明源**:ArgoCD Application / Terraform state / Helm values
- **客户端嵌入依赖**:SDK 内嵌的 Stripe / Twilio / 外部 SaaS

当前接入策略是**每类资源一个 ad-hoc 通道**:K8s 自动、中间件靠 `detect_middleware()` 半自动、其余靠手工建节点。这条路有 5 个不可持续的问题:

| 问题 | 后果 |
|---|---|
| 节点 schema / ID 规范 / 边语义全靠各 connector 内卷 | 3 个月后新人无法独立写 connector,不知道字段所有权 |
| 多源数据合并无统一机制 | K8s 看到的 svc 和 Nacos 看到的 svc 不会自动合;同一 RDS 实例被 Cloud API 和 trace 重复创建 |
| 没有"未知依赖"检测 | trace 看到 `peer.service=stripe.com` 但图里没节点 — 永远丢失,无人察觉 |
| 手工建节点脚本 | 不可持久;变更不感知 |
| 删除策略靠 `discovery_method` 字符串字面值 | 任一 connector 改字面值就破坏其他 connector 的隔离 |

**PRD-005 目标**:把"N 个独立 connector → 各自写 内存孪生层"重构为"**N 个 connector → 发 Fact → Identity Resolver 合并 → 单一 Canonical Graph**",加一条 **Trace-driven Unknown Dependency Queue 做完整度自检**。

---

## 2. 设计原则

### 2.1 没有"单一真理源",只有多通道融合
任何单源都有盲区(云 API 不知道客户端怎么用、K8s 不知道集群外、Trace 看不到没流量的资源、CMDB 有变更延迟)。设计承认**所有通道并存,冲突按规则裁决**。

### 2.2 Fact 是最小单元,不是 Node
Connector 不再直接 `store.upsert_node()`,而是发布"事实":
```
fact(source="cloud_api_aws_rds", observed_at=t1, confidence=0.95,
     claim={type:"RDS", id:"rds:cn-south-1:order-db",
            attrs:{host:"...", port:3306, version:"8.0"}})
```
同一资源多通道发同样事实 → 信心叠加;不同通道冲突 → 走仲裁规则。

### 2.3 Identity Resolution 是核心系统,不是 connector 内部细节
"trace 里看到 `tcp://10.0.1.5:3306` 调用" + "Cloud API 说 RDS `rds-xxx` 的 internal IP 是 `10.0.1.5`" → **同一节点**。这种 cross-source merge **必须有独立模块**,不能藏在 connector 里。

### 2.4 Trace 是"未知依赖"探测器
每个 OTel span 的 `peer.service` / `db.connection_string` / `messaging.url` 是天然的资源指纹。**Trace egress 里出现但拓扑图里没有的目标 = 已知的未知**,自动列入「待解释依赖」队列。**这是检验拓扑完整度的唯一无偏指标**。

### 2.5 GitOps / IaC 是声明意图,运行时观测是事实 — 分两层存
ArgoCD App / Terraform state / Helm values 是"应该有什么";K8sConnector / Cloud API 是"实际有什么"。差异(intent drift)本身就是巡检发现项(InspectionFinding)。

### 2.6 Connector 自描述 (Self-Describing)
每个 connector 启动时声明:`我产哪些 type、拥有哪些字段、不动哪些字段、采集频率`。系统据此自动建 Identity Resolution 规则、自动给冲突字段挑 owner。

---

## 3. 目标架构

```
┌─────────────────────────────────────────────────────────────────┐
│                    数据源(各种现网系统)                          │
├─────────────────────────────────────────────────────────────────┤
│  K8s API     云厂商 API   Nacos    Kong/APISIX   Jaeger        │
│  (集群内)    (RDS/ELB)   (服务簿)  (网关路由)     (调用关系)     │
│  ArgoCD      Terraform   Prom      eBPF                        │
│  (声明意图)  (基础设施声明) (指标)  (网络流量,可选)              │
└────────────────────────┬────────────────────────────────────────┘
                         ▼
              ┌──────────────────────┐
              │ N 个 Connector       │
              │ 各自读一类源          │
              │ 不直接写图            │
              │ 只发 Fact            │
              └──────────┬──────────┘
                         ▼
              ┌──────────────────────┐
              │  Fact 总线           │  ← 新增:进程内 pubsub
              │  TopologyFact{       │     backend/app/topology/
              │    source, type,     │       fact_bus.py
              │    correlation_keys, │
              │    payload, ...      │
              │  }                   │
              └──────────┬──────────┘
                         ▼
       ┌─────────────────┴─────────────────┐
       ▼                                   ▼
┌────────────────┐                  ┌─────────────────┐
│ Identity        │                  │ Unknown Dep     │  ← 新增
│ Resolver        │                  │ Queue           │
│ (合并/创建节点) │                  │ (trace 见过但   │
│ 用 correlation  │                  │  图里没的端点)  │
│ _keys 匹配      │                  └─────────────────┘
└────────┬────────┘                          ↓
         ▼                              暴露给前端
┌────────────────────┐                  /api/v1/topology
│ Canonical Graph    │                  /unknown-deps
│ = 升级后的 内存孪生层     │                  → SRE 看到 → 装新 connector
│ + provenance 字段  │                                          
│ (每个属性谁产的)   │
└──────────┬─────────┘
           ▼
   原有功能照常工作:
   - 7 个巡检视图
   - PRD-001 恢复动作
   - PRD-002 变更事件
   - PRD-003 报告
```

---

## 4. 核心数据模型

### 4.1 TopologyFact 契约

```
# backend/app/topology/models.py

@dataclass
class TopologyFact:
    """所有 connector 都发这个,不直接写 store"""
    source: str               # "cloud_api_aws_rds" / "k8s_connector" / "trace"
    observed_at: datetime
    ttl_seconds: int          # 多久过期触发 stale 检查
    confidence: float         # 0..1,源类型 + 字段一致性算
    fact_type: Literal["node", "edge", "attr", "absence"]
    payload: dict             # 看 fact_type 而定
    correlation_keys: list[str]  # Identity Resolver 用此匹配
```

### 4.2 Correlation Key 规范(强约定)

| 前缀 | 用途 | 例子 |
|---|---|---|
| `ip:` | 网络层 | `ip:10.0.1.5` |
| `endpoint:` | L4 端点 | `endpoint:rds.aws.com:3306` |
| `arn:` | 云资源 ARN/ID | `arn:aws:rds:us-east-1:xxx:db-yyy` |
| `cluster_dns:` | 集群内 DNS | `cluster_dns:order-svc.order.svc.cluster.local` |
| `cluster_node:` | K8s 资源 | `cluster_node:vm-cluster:otel-demo:Pod:cart-xxx` |
| `domain:` | 外部域名 | `domain:api.stripe.com` |
| `process:` | 进程级(eBPF 用) | `process:host-1:12345` |
| `git_url:` | 代码仓(PRD-006) | `git_url:https://gitlab.com/order/order-service` |

任一 key 重叠 → 候选合并;**OTel Semantic Conventions** 是 key 命名的事实标准。

### 4.3 Field Ownership 表

```yaml
# backend/app/topology/field_ownership.yaml
RDS:
  arn:                source=[aws_rds, aliyun_rds]      # 只云 API 能写
  endpoint_host:      source=[aws_rds, gitops]
  engine_version:     source=[aws_rds]
  call_count_5m:      source=[trace]                    # 只 trace 写
  health:             source=[prometheus, cloud_api]    # 多源,新覆盖旧
  current_qps:        source=[trace, cloud_api_metrics]
  owner_team:         source=[gitops, cmdb, code_repo]  # 业务字段
```

不在表里的字段 → 弃写 + warning(防 connector 偷偷加字段)。

### 4.4 节点 provenance 字段(内存孪生层 扩展)

```
# backend/app/datasource/models.py  DataNode 增加
class DataNode:
    ...
    provenance: dict[str, FactProvenance] = field(default_factory=dict)
    correlation_keys: set[str] = field(default_factory=set)

@dataclass
class FactProvenance:
    """单个字段的来源记录"""
    source: str
    observed_at: datetime
    confidence: float
```

每个属性记录谁产的、什么时候产的、可信度多少 — 前端调试 / 字段冲突排查必备。

---

## 5. Identity Resolver 算法

```
# backend/app/topology/identity_resolver.py

async def resolve(fact: TopologyFact):
    # 1) 找所有已存在节点匹配 fact.correlation_keys 任一 key
    candidates = store.find_by_correlation_keys(fact.correlation_keys)

    if len(candidates) == 0:
        return store.create_node(fact.payload, owner_keys=fact.correlation_keys)
    if len(candidates) == 1:
        return store.merge_into(candidates[0], fact)
    # 2) 多匹配 → 冲突
    return arbiter.resolve_collision(candidates, fact)
```

**冲突仲裁规则(默认)**:
1. confidence 较高的 source 胜出
2. 平局看 observed_at,新覆盖旧
3. 仍平局 → 记入 `arbiter_decisions` 表 + 前端列出待人工确认

---

## 6. Unknown Dependency Queue

### 6.1 触发路径
```
Trace 看到调用 endpoint:pay-svc.io:443     →  发 edge Fact
            ↓
Identity Resolver 找不到 endpoint:pay-svc.io:443 对应节点
            ↓
Unknown Dependency Queue 入队:
    {endpoint, count_5m, sample_caller_components,
     first_seen, dns_resolution, asn_lookup}
```

### 6.2 自动富化
```
# backend/app/topology/unknown_dep.py

async def enrich(unknown_dep):
    # 1) DNS 反查 → ASN/CDN/SaaS 推测
    asn = await dns_reverse_lookup(unknown_dep.endpoint)
    # 2) CIDR 落入 → 内网/VPC peering
    if is_private_cidr(unknown_dep.ip):
        unknown_dep.suggestion = "内网资产,建议装 CMDB connector"
    # 3) rdns → cloud provider 命名 → 引导补 Cloud connector
    if "amazonaws.com" in unknown_dep.endpoint:
        unknown_dep.suggestion = "AWS 资产,建议装 aws_rds / aws_elb connector"
    # 4) 代码仓 grep(PRD-006 联动)
    code_hits = await code_repo_search(unknown_dep.endpoint)
    unknown_dep.callers_from_code = code_hits
```

### 6.3 端点
- `GET /api/v1/topology/unknown-deps` — 列表(按 count_5m 降序)
- `POST /api/v1/topology/unknown-deps/{id}/resolve` — 人工标记 + 创建节点
- `POST /api/v1/topology/unknown-deps/{id}/ignore` — 标记为不需要建模(如 health check 探针)

---

## 7. 9 类通道分工

| 通道 | 覆盖资源 | 实现复杂度 | 当前状态 |
|---|---|---|---|
| **Cloud API** | Region/AZ/VPC/ELB/NLB/CDN/RDS/ElastiCache/MQ/S3/KMS/Lambda | 高(每云一套 SDK) | ❌ 全新 |
| **Kubernetes** | NS/Deploy/Pod/Service/**Ingress**/CM/Secret/Node/**PVC**/**HPA** | 中 | ⚠️ 缺 Ingress/PVC/HPA |
| **Mesh xDS** | 服务调用 + retry/timeout/circuit-breaker 配置 + L7 metrics | 中 | ❌ 全新 |
| **Trace+OTel** | 未知依赖 / 客户端嵌的 DB/MQ/Cache / 外部 SaaS | 低-中 | ⚠️ 只用 ChildOf,没看 `db.system` |
| **Network flow** | TCP 连接对 / 跨 VPC 流量 / Pod-IP 反查 | 高(eBPF/cilium hubble) | ❌ 可选 |
| **Config plane** | Nacos/Apollo/Consul 注册的 service + 配置项 + 监听者 | 低 | ❌ 全新 |
| **Gateway admin** | Kong/APISIX/SpringCG 的 routes / upstreams / plugins | 低 | ❌ 全新 |
| **GitOps** | ArgoCD Application / Terraform tfstate / Helm release values | 中 | ⚠️ 只接了 Argo webhook 事件 |
| **CMDB** | 兜底 — 老旧 SaaS / 网络硬件 / 物理机 | 低-中 | ⚠️ 手工 |

**关键洞察**:**Trace + OTel span attrs 是被严重低估的通道**。当前只数 ChildOf 的 span 对,但 span attrs 里有:
- `db.system` / `db.connection_string` / `db.name` → 数据库节点 + 调用边
- `messaging.system` / `messaging.destination` → MQ 节点 + 生产/消费边
- `http.url` / `peer.service` → 外部 SaaS 节点
- `rpc.service` → gRPC 服务依赖

**只升级 `trace_aggregator.py` 这一个文件,就能"免费"发现一大票客户端 SDK 嵌的依赖**。

---

## 8. 工业界对照

| 系统 | 思路 | 我们这里对照 |
|---|---|---|
| **Dynatrace OneAgent** | 装 agent 到每台机器/容器 → 进程树 + 网络 + JVM 钩子 | 我们不走 agent(集群外不可控),靠 OTel + Cloud API 反向推 |
| **Datadog Service Map** | OTel trace + APM 库自动埋点 | 已在这条路上,trace_aggregator 增强即可 |
| **Netflix Atlas+Vizceral** | 内部 service registry + 流量统计 | 需补 Config plane connector(Nacos) |
| **Backstage Catalog** | 人工维护 YAML + plugin pull | GitOps connector 自动同步,把人工最小化 |
| **ServiceNow CMDB Discovery** | 探针扫网段 + WMI/SSH 拉清单 | 走 Cloud API + K8s API,不扫网段 |
| **OpenTelemetry Resource Semantic Conventions** | 标准化资源 attrs | **直接复用,correlation_keys 用 OTel 标准** |

**结论**:走 Datadog/Dynatrace 中间路径 — 不装 agent,但极度依赖 OTel + Cloud API,用 Trace 做盲区检测。

---

## 9. 实施路线 — 6 个 Sprint

| Sprint | 工作量 | 内容 | 验证 |
|---|---|---|---|
| **S1 Fact 总线** | 2 周 | 抽 `fact_bus.py` + `identity_resolver.py` + 内存孪生层 provenance 字段;现有 6 个 connector 改 `publish_fact` 适配层 | 既有 472 后端测试零回归 |
| **S2 Trace 增强**(最高 ROI) | 1 周 | `trace_aggregator.py` 加 `_extract_db_dependencies` / `_extract_msg_dependencies` / `_extract_http_egress`;span attrs 识别 db/messaging/http | 测试集群里 OTel demo 自动发现 Valkey / Kafka 边 |
| **S3 Unknown Dep Queue** | 1 周 | `unknown_dep.py` + 端点 + 前端 `/topology/unknown-deps` 页 | trace 看到外部域名 → 队列内可见 + DNS/ASN 自动富化 |
| **S4 Cloud API 框架 + 首云** | 1 周框架 + 每云 1-2 周 | `connectors/cloud/base_cloud_connector.py` 抽象;华为云 RDS / ELB / DMS Kafka 三个资源接通 | 现网 vm 集群外的托管 MySQL/Kafka 自动入图 |
| **S5 Gateway + Config plane** | 各 1 周 | Kong/APISIX admin API + Nacos Open API | Gateway 路由变更 → ChangeEvent;Nacos 服务下线感知 |
| **S6 GitOps intent** | 2 周 | ArgoCD Application CR + Terraform tfstate 解析 → 声明 Fact;与运行时 Fact 比对 → `intent_drift` InspectionFinding | 现网 Argo App 改 image 但未同步 → 产 finding |

---

## 10. 验收标准

### S1 验收
- [ ] `fact_bus.publish()` + `identity_resolver.resolve()` 单测覆盖 ≥ 20 条
- [ ] 现有 6 个 connector 经 BaseConnector 改造后,全部 472 测试通过
- [ ] 内存孪生层 节点带 `provenance` 字段,GET `/datasource/nodes/{id}` 返回字段所有权信息

### S2 验收
- [ ] OTel demo 集群运行 5 分钟后,图谱里出现 `cart -CALLS-> valkey` `checkout -PRODUCES_TO-> kafka:orders` 边
- [ ] 边 properties 含 `call_count_5m / p99_ms / discovery_method=trace`

### S3 验收
- [ ] 在测试环境模拟一个调用外部域名(`curl https://httpbin.org/get`)
- [ ] `/api/v1/topology/unknown-deps` 返回该 endpoint,带 DNS/ASN 推测 + count

### S4 验收
- [ ] 华为云 vm 集群对应 region 的 RDS / DMS Kafka 实例,uvicorn 启动后 5 分钟内入图
- [ ] 节点 properties 含 `host / port / engine_version / arn`
- [ ] PRD-001 `kill_query` 真模式按 target.cluster_id 路由到正确实例

### S5 验收
- [ ] Nacos 服务实例上下线 → 内存孪生层 Service 节点 endpoints 列表实时更新
- [ ] Kong 路由变更 → 产 ChangeEvent(source=gateway_admin)

### S6 验收
- [ ] ArgoCD App 改 image tag 但 sync 失败 → 产 `intent_drift` InspectionFinding
- [ ] Finding 含 declared_value / actual_value / repo_url / commit_sha

---

## 11. 风险与缓解

| 风险 | 缓解 |
|---|---|
| Fact 总线引入 latency | 进程内 pubsub(asyncio.Queue),不过网络;benchmark ≤ 1ms/fact |
| Identity Resolver 误合并 | 仲裁前置:correlation key 必须严格匹配(不做模糊);冲突走人工确认队列 |
| Cloud API rate limit | 30s-5min 轮询(可配),分页 + ETag 增量;超限退避 |
| Trace 增强引入 trace_aggregator 性能问题 | span attrs 解析走 selective extract(只看预设 keys);批处理而非流式 |
| GitOps connector 解析失败(Terraform state 加密 / 跨账号) | best-effort + 失败标 `parse_error` finding,不阻塞主流程 |
| 老 connector 改造改坏既有功能 | 适配层而非重写:`store.upsert_node()` → 内部产 Fact 走总线,接口签名不变;472 测试当回归基线 |

---

## 12. 不做(本期外)

| 能力 | 延后到 |
|---|---|
| 真 agent(OneAgent 类) | 永久不做 — 集群外资产不可控,违背"非侵入"原则 |
| eBPF Network flow | Phase 4(可选,看运维诉求) |
| Identity Resolution 多账号合并(跨 tenant) | Phase 4 |
| Fact 总线持久化 / 重放 | Phase 4(目前内存即可,connector 重连补齐) |
| 自动学习 correlation 规则 | Phase 5(LLM 辅助,慎用) |

---

## 13. File Map(实施后)

```
backend/app/
├── topology/                            # PRD-005 新模块
│   ├── models.py                        # TopologyFact + FactProvenance
│   ├── fact_bus.py                      # 进程内 pubsub
│   ├── identity_resolver.py             # 合并算法 + 仲裁
│   ├── unknown_dep.py                   # Unknown Dependency Queue + 富化
│   ├── field_ownership.yaml             # 字段所有权配置
│   └── arbiter.py                       # 冲突仲裁
├── datasource/
│   ├── store.py                         # 内存孪生层:加 correlation_index + provenance
│   ├── models.py                        # DataNode:加 provenance / correlation_keys
│   └── connectors/
│       ├── base.py                      # BaseConnector:加 publish_fact 适配
│       ├── cloud/                       # S4:Cloud API connectors
│       │   ├── base_cloud_connector.py
│       │   ├── huawei_rds.py
│       │   ├── huawei_dms_kafka.py
│       │   └── huawei_elb.py
│       ├── gateway/                     # S5:Gateway admin connectors
│       │   ├── kong_connector.py
│       │   └── apisix_connector.py
│       ├── config_plane/                # S5:服务注册中心
│       │   └── nacos_connector.py
│       ├── gitops/                      # S6:声明意图
│       │   ├── argocd_connector.py
│       │   └── terraform_state.py
│       └── trace_aggregator.py          # S2:span attrs 增强
└── routers/
    └── topology.py                      # /api/v1/topology/unknown-deps 等
```

---

## 14. 一句话总结

PRD-005 把架构从"**N 个独立 connector → 各自写 内存孪生层**"重构为"**N 个 connector → 发 Fact → Identity Resolver 合并 → 单一 Canonical Graph**",加 **Trace-driven Unknown Dependency Queue 做完整度自检**。Sprint 2(trace_aggregator 增强单文件)ROI 最高,先做;Sprint 4(Cloud API)解决 ELB / RDS / Kafka 等集群外资产盲区。
