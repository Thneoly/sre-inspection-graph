# 14 — 长期技术战略:Python → Rust + WASM 渐进迁移

## 0. 上下文 / 为什么写这份

这个项目是**副业**(主业兜底无短期收入压力),但目标是 **长期成果**(技术资产、架构示范作、可能的未来商业化前奏)。团队具备**成熟的 Rust + WASM/WASI 工程经验**和**可借鉴的参考项目**。

商业团队的"不要重写"铁律(Joel)在这个场景下**只适用一半** — 时间约束消失了,但 **代码里 embedded 的知识(12,732 LOC + 472 测试里的边界 case)仍然不能丢弃**。

本文落档:**未来 24 个月**沿着 **Strangler Fig** 路径,把 Python 平台逐步替换为 **Rust + WASM 引擎**,**不做 clean-sheet rewrite**。

> 这是 **决策文档**,不是设计文档。详细技术 schema 在 [`15-data-contract-spec.md`](./15-data-contract-spec.md);PRD-005/006 实现路径在各自 PRD 里。

## 1. 决策摘要(TL;DR)

| 项 | 决策 |
|---|---|
| 整体语言路线 | Python → Rust + WASM(渐进) |
| 迁移方法 | **Strangler Fig**(老 Python 不动,新模块 Rust 原生,逐模块替换) |
| 时间窗 | 24 个月(T+0 → T+24mo) |
| 终态 | Rust + WASM 核心引擎,Python 仅 CLI/scripts(或完全退役),前端 TS 不动 |
| **数据契约(三层)** | **WIT**(WASM↔host) + **Arrow Flight**(fact 数据面) + **JSON/REST**(控制面 + 业务 API) |
| Neo4j 角色 | 退化为审计/持久化备份,**热路径全在 Rust 内存** |
| 新 PRD 实现 | PRD-005/006 直接 Rust+WASM 原生写,不再写 Python 版 |
| 老 PRD 处理 | PRD-001/002/003/004 现状保留,T+12mo 后逐模块迁 |
| 测试策略 | Contract testing 框架,Python 测试转行为规约,Rust 实现复刻通过 |
| 退出条件 | 见 §8 |

## 2. 战略目标

按优先级:

1. **架构差异化** — 拿到 **WASM-native SRE 平台** 这个定位,这是 Python 生态做不到的。参考 Envoy / Fastly / Suborbital。
2. **技术资产质量** — 24 个月后的代码库应当是"愿意挂在 GitHub 给同行看"的水平,**不只是能跑**。
3. **个人/团队能力沉淀** — Rust + WASM + SRE 三个领域交叉的实战经验,这是稀缺技能。
4. **长期可维护性** — 副业产能有限,**架构必须经得起 context switch**(放两周再回来还能继续写)。
5. **可演示性** — 每 Phase 末尾应有 **公开可演示的成果**(blog / demo / 仓库 release),这是副业项目维持动力的关键机制。

**非目标**(明确不追求):

- 短期可商业化(2 年内不卖)
- 抢市场速度
- 成为最快/最便宜的 SRE 工具
- 兼容所有云厂商(选 1-2 个深度做)

## 3. 整体架构终态(T+24mo 视图)

```
┌─────────────────────────────────────────────────────────────┐
│  Frontend (TS + Cytoscape) ── 保留                           │
└──────────────────────┬──────────────────────────────────────┘
                       │ REST / JSON (OpenAPI auto)
┌──────────────────────▼──────────────────────────────────────┐
│  topology-engine (Rust)                                     │
│   ├── axum REST(控制面 + 视图查询)                          │
│   ├── arrow-flight gRPC(fact 数据面)                       │
│   ├── wasmtime runtime(WASM connector / rule / handler)     │
│   ├── Canonical Graph Store(内存,Arrow-backed columnar)    │
│   ├── Identity Resolver(DataFusion SQL 查 Arrow)            │
│   ├── Recovery Engine(沿 PRD-001 contract 迁)               │
│   └── Report Engine(可选迁,Jinja2 留 Python 也行)          │
└──────────────────┬───────────────────┬──────────────────────┘
                   │ WIT (Component    │ Arrow Flight
                   │  Model ABI)       │ (跨进程 connector)
┌──────────────────▼────────────┐   ┌─▼──────────────────────┐
│ WASM Modules                  │   │ External Agents        │
│  • k8s-connector.wasm         │   │  • cloud-agent(Go/Rust)│
│  • prom-connector.wasm        │   │  • on-prem agent       │
│  • jaeger-connector.wasm      │   │                        │
│  • flagd-connector.wasm       │   │                        │
│  • cloud-connector.wasm       │   │                        │
│  • coderepo-connector.wasm    │   │                        │
│  • inspection-rules/*.wasm    │   │                        │
│  • recovery-handlers/*.wasm   │   │                        │
└───────────────────────────────┘   └────────────────────────┘
                   │
                   │ 后台异步持久化(降级)
                   ▼
              ┌──────────┐
              │  Neo4j   │  仅审计/备份/复杂图查询时用
              └──────────┘
```

**关键变化**:
- DSS 内存层从 Python dict → Rust Arrow columnar(向量化扫描)
- Connector 从 Python class → WASM module(热加载)
- Recovery handler 从 Python class → WASM module(沙箱)
- Inspection rule 从 Python regex → WASM(任何语言可编译过来)
- Neo4j 从热路径 → 冷归档

## 4. 三层数据契约(关键决策)

**不用 protobuf**。三层各司其职:

| 层 | 协议 | 用途 | 工具链 |
|---|---|---|---|
| **A. WASM ↔ host** | **WIT** (Component Model) | 强类型边界,小消息高频,零开销 | `wit-bindgen` |
| **B. Fact 总线数据面** | **Arrow Flight** | 高吞吐流式 tabular 数据,零拷贝 | `arrow-flight` (Rust) + `pyarrow.flight` (Python) |
| **C. 控制面 + 业务 API** | **JSON over REST** | 低频请求/响应,易调试,前端友好 | `axum` (Rust) + FastAPI (Python) + 自动 OpenAPI |

**为什么不 protobuf**:
- 小消息走 WIT 已足够,无需 protoc
- 大批量走 Arrow Flight 零拷贝,胜过 protobuf marshal
- 控制面 JSON 调试更友好,前端白嫖 OpenAPI
- **少装一个 build 工具链(protoc),副业可持续性 +1**

**衍生收益**:Arrow → Parquet 落对象存储几乎零成本,未来历史 fact 归档/分析直接接 DuckDB/DataFusion,不再设计存储格式。

详细 schema 见 [`15-data-contract-spec.md`](./15-data-contract-spec.md)。

## 5. Strangler Fig 迁移节奏(24 个月)

每个 Phase 都设 **公开可演示节点**,作为副业项目的外部 commitment 机制。

### Phase 0 — 决策固化(T+0 ~ T+2 周) ▶ 进行中

- [x] doc/11/12 PRD-005/006 规划落档
- [x] doc/13 端到端剧本
- [ ] **doc/14(本文)+ doc/15 数据契约 落档**
- [ ] 修改 doc/11 PRD-005,把 Rust+WASM 路径列为首选实现
- [ ] 公开 GitHub project board,设 milestone 节点
- [ ] **demo 节点**:文档体系完整可读

### Phase 1 — Rust 引擎骨架(T+0 → T+3mo)

- [ ] 新仓 `topology-engine`(Cargo workspace)
- [ ] 集成 wasmtime + WIT 工具链
- [ ] `axum` REST 骨架(`/health`, `/facts`, `/connectors`)
- [ ] `arrow-flight` server 骨架,定义 fact schema(同 doc/15)
- [ ] **第一个 WASM connector**:把 `k8s_connector.py` 等价逻辑用 Rust 写,编译成 `.wasm`,通过 WIT 接口装载
- [ ] Python FastAPI 通过 REST 调 Rust 拿 fact / 状态,e2e 跑通
- [ ] **demo 节点**:blog "Building a WASM-native SRE Platform — Part 1: 引擎骨架"

### Phase 2 — PRD-005 Rust 原生实现(T+3 → T+9mo)

- [ ] Fact 总线 Rust 实现(Arrow RecordBatch 内部表示)
- [ ] Identity Resolver(DataFusion SQL 做去重 / merge)
- [ ] Unknown Dependency Queue + 代码仓 enrichment hook
- [ ] WASM 化 Prometheus / Jaeger connector
- [ ] Cloud API connector(华为云 / AWS 至少一家)走 Arrow Flight 上传
- [ ] DSS canonical store 双写过渡:Python DSS + Rust store 并存,行为 diff 校验
- [ ] **demo 节点**:blog "Part 2: Trace 看到的 Stripe 被 Rust 引擎纳管,5 分钟入图"

### Phase 3 — PRD-006 代码仓 + WASM 规则引擎(T+9 → T+15mo)

- [ ] code_repo_connector(WASM 模块)
- [ ] 业务规则 WASM 引擎(用户写 rule.wasm,host 沙箱跑)
- [ ] PR/MR webhook → ChangeEvent 扩展
- [ ] 规则市场 PoC(.wasm 文件分发机制)
- [ ] **demo 节点**:blog "Part 3: 用户自定义巡检规则,WASM 沙箱执行"
- [ ] 可能的对外演讲/分享(QCon / GIAC / GopherChina-Rust track)

### Phase 4 — 老 Python 模块逐个迁(T+15 → T+24mo)

按风险升序迁移:

1. **MetricSnapshot / AlertEvent service**(T+15-16):简单,纯数据
2. **ChangeEvent service**(T+16-18):中等,有 Neo4j 双写
3. **Recovery Engine**(T+18-21):复杂,要保留所有 472 测试行为
4. **Report Engine**(T+21-23):Jinja2 换 Tera,或留 Python 永不迁
5. **FastAPI 路由层**(T+23-24):可选换 axum,或保留 Python 作为兼容层

每一步:**老 Python 模块跑业务 + 新 Rust 模块影子跑,输出 diff,zero diff 持续一周才切流量**。

- [ ] **demo 节点**:blog "Part 4: Python 模块如何安全退役"
- [ ] **终态 demo**:Rust 100% 核心,WASM 插件生态完整

## 6. 接口契约策略

为了 Strangler Fig 安全平移,每个被迁移模块必须先做:

1. **行为规约文档化** — 把现有 Python 模块的对外行为(API 响应、DSS 变化、Neo4j 写入、事件发布)写成 YAML contract
2. **现有 Python 测试转 contract test** — 472 个测试逐个映射到 contract,**Python 实现 + Rust 实现都跑同一份 contract**
3. **双跑验证** — Python 主、Rust 影子,所有 contract test 双方都通过 + 实际流量输出 diff 为零 ≥ 1 周
4. **流量切换** — feature flag 切到 Rust,Python 模块保留 1 个月回退余地
5. **Python 退役** — 移除代码,contract 归档

Contract 格式细节见 [`15-data-contract-spec.md`](./15-data-contract-spec.md) §6。

## 7. Phase 验收标准

每 Phase 验收必须满足:

| 维度 | 标准 |
|---|---|
| **功能** | 对应 PRD 验收准则全部通过 |
| **测试** | Rust 部分 ≥ 80% line coverage,关键路径 hypothesis/proptest 覆盖 |
| **性能基线** | 与 Python 版本对比,关键路径 ≥ 5× 提升(否则只是浪费精力) |
| **可演示** | 有 blog / demo video / public release |
| **文档** | 对应模块的 doc/PRD 更新到与代码一致 |
| **回归** | 已迁移模块的旧 Python 测试零回归(通过 contract test) |

## 8. 退出条件(什么情况下停止此计划)

副业项目必须有诚实的退出机制,避免变成"沉没成本陷阱"。任一条触发就重新评估:

| 触发 | 应对 |
|---|---|
| 连续 3 个月主业占满,无法投入项目 | **暂停**,公开仓库 archive,保留所有 doc/code |
| Phase 1 结束时(T+3mo)WASM connector 无法稳定跑 | **回退**:放弃 WASM 路径,只用 Rust 不做 WASM 插件化 |
| Phase 2 结束时(T+9mo)Rust 引擎性能未达 Python 5× | **重新评估**:可能是设计问题不是语言问题,审视架构 |
| Rust 主流 crate(wasmtime / arrow-rs / tokio)出现重大破坏性变更且无平滑迁移路径 | **冻结版本**,等待生态稳定 |
| 团队 Rust 经验丢失(人员变动) | **冻结新模块开发**,保护已迁移部分 |
| 24 个月后未到达 T+15mo 实际进度 | **诚实评估**:是否方向有问题,or 接受 36 个月节奏 |
| 个人兴趣转移 | **不强求**,公开 archive,文档保留 |

**重要**:退出不是失败。半成品如果有 doc 完备,本身就是技术资产。

## 9. 风险登记

| 风险 | 影响 | 缓解 |
|---|---|---|
| 双语言代码库长期维护成本 | 中 | gRPC/REST 边界 clean,各自独立部署/测试;Strangler Fig 终点是单语言 |
| 24 个月跨度兴趣维持 | **高(副业最大风险)** | 公开 milestone + blog + 自用驱动;每 Phase 有可见产出 |
| Rust + WASM 生态 breaking change | 中 | 锁主流 crate minor 版本,半年评估升级;WIT 选 Component Model(2024 稳定) |
| 个人 code review 缺失 | 中 | `clippy --deny warnings` + `cargo audit` + `cargo deny`;关键 PR 拉同行 review |
| 过度设计(无现网压力) | **高** | 自己当 customer:**自用 SRE 工具**,真在生产环境跑(主业相关或开源他用) |
| Neo4j 退化为冷存储后想用其图查询能力 | 低 | 保留 Python Neo4j adapter,需要时调用,不强迫 Rust 重做图查询 |
| 老 Python 472 测试在迁移中丢失 | 高 | Contract test 框架强制(见 §6) |
| Frontend 长期跟不上后端演进 | 中 | 后端 API 用 OpenAPI 自动出 TS client,前端零成本跟进 |

## 10. 不做(本战略)

明确排除:

| 能力 | 原因 |
|---|---|
| 整体 clean-sheet rewrite | Chesterton's Fence — 老代码里的知识不能丢 |
| 用 Go(替代 Rust) | 团队已具 Rust+WASM 经验,换 Go 是浪费;WASM 生态 Rust 更前沿 |
| 自研 WASM runtime | 用 wasmtime,Bytecode Alliance 标准实现 |
| 自研列存格式 | 用 Arrow,事实标准 |
| 改前端语言 | TS + Cytoscape 已稳,前端不是瓶颈 |
| Neo4j 改图数据库 | Neo4j 退到冷路径后,选型不重要 |
| 24 个月内做 IM 推送 / PDF 报告 / AI 推荐 | 这些都是 v3 能力,优先底座 |

## 11. 成功度量(T+24mo 时)

主观但具体:

| 度量 | 目标 |
|---|---|
| Rust 核心引擎 LOC | 15-25k(基本对齐 Python 现状,部分模块更小) |
| WASM 插件数 | ≥ 8 个(6 connector + 2 rule/handler) |
| Contract test 通过率 | 100%(行为完全对齐迁移前) |
| 公开 blog 系列 | ≥ 4 篇,Part 1-4 |
| GitHub stars | ≥ 100(weak signal,不强求) |
| 自用价值 | 主业或开源环境真在跑,且我自己愿意每天看 |
| 个人技术深度 | Rust + WASM + SRE 三领域交叉,**这是最重要的隐性 ROI** |

## 12. 立即下一步

T+0(本周末):
1. ✅ 写完本文 + doc/15
2. [ ] 改 doc/11 PRD-005,把"实现选型"章节改为 Rust+WASM 首选
3. [ ] 建 GitHub project board,milestone 拆到 Phase 1 周级
4. [ ] 写 Phase 1 "Build a WASM-native SRE Platform" 系列第 1 篇 outline

T+1 周:
- [ ] 新建 `topology-engine` Cargo workspace
- [ ] `Cargo.toml` 锁定 crate 版本基线(见 doc/15 §7)
- [ ] hello-world WASM connector 跑通

## 13. 相关文档

- **本文上游**:[`13-story-unknown-dep-stripe.md`](./13-story-unknown-dep-stripe.md)(为什么 WASM 是战略)
- **本文下游**:[`15-data-contract-spec.md`](./15-data-contract-spec.md)(三层契约详细 schema)
- **架构上下文**:[`11-PRD-005-...`](./11-PRD-005-universal-topology-service.md) / [`12-PRD-006-...`](./12-PRD-006-code-repo-source.md)
- **现状参照**:[`10-product-gap-analysis.md`](./10-product-gap-analysis.md)(MVP 100% 起点)
- **导航**:[`00-README.md`](./00-README.md)

---

**签字**(决策快照):

| 项 | 状态 |
|---|---|
| 决策时间 | 2026-06-23 |
| 决策范围 | 24 个月技术路线 |
| 决策依据 | 副业属性 + 长期成果 + 团队 Rust/WASM 经验 + WASM 战略价值 |
| 下一次重评 | T+3mo(Phase 1 结束) |
