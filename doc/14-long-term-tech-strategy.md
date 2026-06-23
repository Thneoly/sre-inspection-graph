# 14 — 长期技术战略:Supervised Rewrite + Tauri 桌面化

## 0. 上下文 / 为什么写这份

这个项目是**副业**(主业兜底无短期收入压力),目标是 **长期技术资产 / 架构示范作 / 个人自用工具**。团队具备**成熟的 Rust + WASM/WASI 工程经验**和**可借鉴的参考项目**。

商业团队的"不要重写"铁律在**副业场景部分失效** — 时间约束消失了,持续交付不是必需,但 **代码里 embedded 的知识(12,732 LOC + 472 测试里的边界 case)仍然不能丢弃**。

> 本文是 doc/14 的 v0.2。v0.1 曾推荐 Strangler Fig 渐进迁移,**复盘后否决**:Strangler 的核心好处(零停机持续交付)对副业不成立,代价(桥接 / 双语维护 / 心智切换)却照收。**实测 Strangler 多花 ~17 个月换不需要的能力**。改为 **Supervised Rewrite** 路线。

## 1. 决策摘要(TL;DR)

| 项 | 决策 |
|---|---|
| 整体语言路线 | Python → Rust + WASM(**全量替换**,Supervised Rewrite) |
| 迁移方法 | 全新 Rust 仓 + WASM 模块;**Python 仓改名 `reference/`,read-only,本地 dev 作 oracle**;完成后 `git rm` |
| 产品形态 | **Tauri 2.x 桌面应用**(单二进制,本地优先,跨平台)— **不**走 SaaS Web 默认路径 |
| 时间窗 | **12 个月**(T+0 → T+12mo,加 2 个月 buffer 到 v1.0) |
| 终态 | Rust + WASM 核心引擎 + Tauri 桌面 app + 可选 headless CLI(团队/SaaS 模式) |
| 前端 | **沿用 React 18 + TS + AntD 5 + Cytoscape.js**,迁移宿主到 Tauri webview;UI 代码 ~90% 复用 |
| 数据契约 | WIT(WASM 边界) + Tauri commands(UI↔Rust,本地 IPC) + Arrow / SQLite / Parquet(存储) + REST/Flight(仅 headless 模式) |
| 本地存储 | **SQLite + Parquet 默认**;Neo4j 可选(headless 模式给团队用) |
| 测试策略 | Rust 单测试栈;Python `reference/` 作行为 oracle,关键路径转 Rust contract test |
| 退出条件 | 见 §8 |

## 2. 战略目标(优先级)

1. **个人/团队自用工具** — 主业 SRE 工作中真在用,这是最大动力来源
2. **架构差异化** — WASM-native + 桌面优先 SRE 工具(类比 k9s + 图形化),非主流但有空间
3. **技术资产质量** — 12 个月后代码库应"愿意挂在 GitHub 给同行看"
4. **个人能力沉淀** — Rust + WASM + Tauri + SRE 四领域交叉实战经验
5. **副业可持续** — 必须经得起 context switch(放 2 周回来还能写)
6. **公开演示价值** — 每 3-4 个月一篇 blog,milestone 驱动

**非目标**:

- 短期商业化(2 年内不卖)
- 抢市场速度
- 多租户 SaaS(后期通过 engine-cli 可扩,但不是核心)
- 浏览器直接访问(Tauri 桌面专属)
- 移动端

## 3. 整体架构终态(T+12mo 视图)

```
┌─────────────────────────────────────────────────────────────┐
│  Tauri Desktop App  ── 单二进制,跨平台(macOS/Linux/Win)  │
│                                                             │
│   ┌─────────────────────────────────────────────────┐      │
│   │  Webview                                         │      │
│   │   React 18 + TS + AntD 5 + Cytoscape.js          │      │
│   │   ├── 6 巡检视图 + 4 PRD 视图                    │      │
│   │   ├── 故障模拟 / 报告 / 审批中心 / 恢复链         │      │
│   │   └── 通过 invoke('cmd_name', args) 调 Rust     │      │
│   └────────────────────┬────────────────────────────┘      │
│                        │ Tauri IPC(进程内 JSON)          │
│   ┌────────────────────▼────────────────────────────┐      │
│   │  tauri-commands(薄包装层,~500 LOC)            │      │
│   └────────────────────┬────────────────────────────┘      │
│                        │ 直接 fn 调                       │
│   ┌────────────────────▼────────────────────────────┐      │
│   │  engine-core(共用 Rust 内核)                   │      │
│   │   ├── Fact 总线 + Identity Resolver(DataFusion)│      │
│   │   ├── Canonical Graph Store(Arrow 内存表)      │      │
│   │   ├── wasmtime runtime(WASM 模块宿主)          │      │
│   │   ├── Recovery / ChangeEvent / Reports          │      │
│   │   └── engine-storage(SQLite + Parquet)          │      │
│   └────────────────────┬────────────────────────────┘      │
│                        │ WIT (Component Model)            │
│   ┌────────────────────▼────────────────────────────┐      │
│   │  WASM Modules                                     │      │
│   │  • k8s.wasm / prom.wasm / jaeger.wasm /          │      │
│   │    flagd.wasm / coderepo.wasm                    │      │
│   │  • threshold-rule.wasm / slo-rule.wasm           │      │
│   │  • custom-recovery-handler.wasm                  │      │
│   └─────────────────────────────────────────────────┘      │
│                                                             │
│  本地存储:                                                  │
│   • SQLite ── metadata(executions / approvals / changes)   │
│   • Parquet ── fact 历史归档(可选)                        │
│   • ~/.config/sre-graph/ ── kubeconfig / Prom URL 等        │
└─────────────────────────────────────────────────────────────┘
                          ▲ 直接连(用户机器上)
                          │
                  ┌───────┴────────┐
                  │  User's K8s,  │
                  │  Prometheus,   │
                  │  Jaeger, Git   │
                  └────────────────┘


┌─────────────────────────────────────────────────────────────┐
│  engine-cli  ── 可选,headless 二进制(团队/SaaS 模式)      │
│   ├── 共用 engine-core                                      │
│   ├── REST + Arrow Flight server                            │
│   └── 可挂 Neo4j 作中心存储                                 │
└─────────────────────────────────────────────────────────────┘


┌─────────────────────────────────────────────────────────────┐
│  reference/  ── Python 老仓,本地 dev only,DO NOT DEPLOY    │
│   └── 跑老 FastAPI,curl 对比新 Rust 行为                    │
└─────────────────────────────────────────────────────────────┘
```

**关键设计点**:

- **默认路径无网络栈** — Tauri webview ↔ Rust 是 IPC,**不走 HTTP**。CORS / dev proxy / auth 这些问题消失。
- **engine-core 是单一内核**,Tauri 嵌入 / CLI 包它,两个交付物共代码。
- **Neo4j 退化为可选** — 桌面用户不装,headless 模式可挂。
- **WIT 是 WASM 边界唯一契约**;Tauri commands 是 UI 边界契约;REST/Flight 只在 headless 路径。

## 4. 三层数据契约(更新版,Tauri 优先)

| 层 | 协议 | 边界 | Tauri 模式 | Headless 模式 |
|---|---|---|---|---|
| A | **WIT** (Component Model) | WASM ↔ host | ✅ | ✅ |
| B | **Tauri commands**(JSON IPC + tauri-specta TS 类型生成) | webview ↔ Rust | ✅ **首选** | ❌ 不需要 |
| B' | **REST + Arrow Flight** | 外部客户端 ↔ engine-cli | ❌ 不需要 | ✅ **首选** |
| C | **Arrow RecordBatch**(内存) + **Parquet**(归档) | engine 内部 | ✅ | ✅ |

**简化**:Tauri 路径无 protobuf / 无 REST / 无 Arrow Flight 跨进程 RPC。所有数据流量进程内传递。

详细 schema 见 [`15-data-contract-spec.md`](./15-data-contract-spec.md) + [`17-tauri-desktop-architecture.md`](./17-tauri-desktop-architecture.md)。

## 5. Supervised Rewrite 节奏(12 个月)

每 Phase 末有公开 demo,作为副业项目外部 commitment 机制。

### Phase 0 — 决策固化(T+0,1-2 周) ▶ 进行中

- [x] doc/11/12 PRD-005/006 规划
- [x] doc/13 端到端剧本
- [x] doc/14(本文,v0.2)+ doc/15 + doc/16 + doc/17 落档
- [ ] `backend/` → `reference/` 改名,加 DO NOT DEPLOY README
- [ ] `frontend/` → `desktop/` 改名(留作 Tauri 集成)
- [ ] 顶层 Cargo workspace 骨架(`engine/` + `modules/`)
- [ ] GitHub project board + Phase milestone 公开

### Phase 1 — Tauri + engine 最小可跑(T+0 → T+1mo)

- [ ] Tauri 2.x app 骨架,React + AntD + Cytoscape 加载
- [ ] engine-core + wasmtime hello world,1 个 invoke 调通
- [ ] WIT 接口完整定义(types / connector / rule / handler)
- [ ] 第一个 WASM connector:`k8s.wasm` 对照 `reference/app/datasource/connectors/k8s_connector.py` 实现等价逻辑
- [ ] 1 个最小视图:打开 app 看到 1 张 mock 拓扑图
- [ ] **Demo + Blog Part 1**:"WASM-native SRE Desktop — 引擎骨架"

### Phase 2 — PRD-005 在 Rust 原生 + 真数据(T+1 → T+4mo)

- [ ] Fact 总线 + Identity Resolver(DataFusion SQL)+ Unknown Dep Queue
- [ ] 5 个 connector WASM 化(k8s / prom / jaeger / flagd / k8s_events)
- [ ] Cloud API connector(华为云或 AWS)— Arrow Flight 客户端,通过 engine-cli 也能用
- [ ] Tauri 视图迁:topology / connectors / unknown-deps
- [ ] SQLite + Parquet 本地存储就位
- [ ] **Demo + Blog Part 2**:"连真实 k8s 集群 + trace 看到的 Stripe 自动入图"

### Phase 3 — PRD-001/002 + PRD-006(T+4 → T+8mo)

- [ ] Recovery 引擎复刻(8 action + dry-run + 审批 + 回滚 + 跨集群 + 自动验证 + 动作链)
- [ ] ChangeEvent 引擎复刻(propagation + correlated + frequency + alert correlation)
- [ ] code_repo_connector(WASM)+ 业务规则 WASM 引擎(PRD-006 S1+S2)
- [ ] 自定义 recovery handler WASM(PRD-001 Phase 3 解锁项)
- [ ] Tauri 视图迁:recovery / change-timeline / approvals / chains / config-impact
- [ ] **Demo + Blog Part 3**:"用户自定义规则 + 自定义恢复动作,WASM 沙箱跑"

### Phase 4 — PRD-003/004 + 收尾(T+8 → T+12mo)

- [ ] Report 引擎(application_health / cluster_overview / incident_report 三模板)
- [ ] APScheduler → tokio-cron-scheduler;SMTP 邮件 → lettre
- [ ] connector 状态页 + 故障模拟视图迁完
- [ ] 跨平台打包:macOS dmg / Linux AppImage+deb / Windows msi
- [ ] tauri-updater 自动更新机制
- [ ] **`git rm -r reference/`** 仪式
- [ ] **Demo + Blog Part 4**:"v1.0 release,跨三平台桌面 app"

### Phase 5 — Buffer + 社区(T+12 → T+14mo)

- [ ] GitHub release v1.0
- [ ] 写一篇综述长文 / 技术演讲投稿(QCon Rust / GIAC 等)
- [ ] 文档 / 教程完整化
- [ ] 收 issue 改 bug

## 6. Supervised 工作流(取代 Strangler Fig)

**每个被迁移模块的标准步骤**:

```
1. 翻 reference/ 找到对应 Python 模块 + 测试,通读理解行为
2. 写 Rust 实现(可以重设计,不必照搬结构)
3. 写 Rust 测试 — 用 reference/ 的测试作行为规约参考,但不强制 1:1 转译
   • 仍有价值的边界 case → 转 Rust test
   • Python 特有 case(asyncio bridge / FastAPI 集成)→ 弃
4. 开发期对照验证:
   • 终端 1:cd reference && uv run uvicorn ... — 跑老 Python
   • 终端 2:cargo run --bin engine-server — 跑新 Rust
   • 对同一输入 curl,diff 输出,直到一致
5. Rust 通过后,reference/ 对应模块标记为「已复刻」(README 表格)
6. Phase 4 末尾,确认所有模块复刻完,git rm -r reference/
```

**reference/ 目录纪律**:
- ❌ 不接受 feature 改动
- ❌ 不接受 bug 修复(除非阻塞 dev 验证)
- ✅ 接受最小化兼容补丁(让 reference 跑得起来即可)
- ✅ 接受 README 表格更新(标记哪些模块已复刻)

## 7. Phase 验收标准

| 维度 | 标准 |
|---|---|
| **功能** | 对应 PRD 验收准则全过;若该 Phase 复刻老 PRD,行为与 reference 对齐 |
| **测试** | Rust 单测 + 集成测覆盖关键路径;hypothesis/proptest 用于核心算法(BFS / Identity Resolver) |
| **性能** | 与 Python reference 对比:关键路径 ≥ 5× 提升,内存 ≤ 30% |
| **可演示** | Tauri app 跑得起来,操作流畅,有 blog / demo video / GitHub release tag |
| **文档** | 对应 PRD doc 更新到与代码一致 |
| **平台** | macOS 优先(开发机)+ Linux(用户),Windows 在 Phase 4 末尾兜底 |

## 8. 退出条件

副业项目必须诚实的退出机制。任一触发重新评估:

| 触发 | 应对 |
|---|---|
| 连续 3 个月主业占满,无法投入 | **暂停**,公开仓库 archive,所有 doc/code 保留 |
| Phase 1 结束(T+1mo)Tauri + engine 无法稳定跑 | **重新评估技术选型**,可能回退到纯 Web(放弃 Tauri) |
| Phase 2 结束(T+4mo)Rust 性能未达 reference 5× | **审视架构**(可能是 Arrow / DataFusion 误用) |
| Tauri / wasmtime / arrow 主流 crate 出现重大破坏性变更 | **冻结版本**,等生态稳定 |
| 团队 Rust 经验丢失 | **冻结新模块**,保护已迁移部分 |
| 12 个月后未到 Phase 3 实际进度 | **诚实评估**,接受 18 个月节奏 or 调减范围 |
| 个人兴趣转移 | **不强求**,公开 archive,文档保留 |

**退出 ≠ 失败**。半成品 + 完整 doc 仍是技术资产。

## 9. 风险登记

| 风险 | 影响 | 缓解 |
|---|---|---|
| **12 个月跨度兴趣维持**(副业最大风险) | 高 | 公开 milestone + blog + 自用驱动;每 Phase 有可见产出 |
| Tauri 生态不如 Web 成熟 | 中 | Tauri 2.x 2024 GA,生态正起;无法解时回退 Web(engine-cli + browser) |
| 桌面 app 用户安装意愿 | 中 | 自用第一,他用第二;签名 + 自动更新降摩擦 |
| Cytoscape.js 在 Tauri webview 性能 | 低 | webview 现代,性能与浏览器同;预 Phase 1 验证一次 |
| Rust + WASM 生态 breaking change | 中 | 锁主流 crate minor;半年评估升 |
| Python reference 漂移 | 中 | 显式 read-only 纪律(§6)+ README 标记 |
| 472 测试转译耗时 | 中 | 不全转,只转有价值的;FastAPI/Neo4j 集成测试弃 |
| 单人 code review 缺失 | 中 | `clippy --deny warnings` + `cargo audit` + `cargo deny`;关键 PR 拉同行 review |
| 过度设计(无现网压力) | 高 | 强制自用 — 主业 SRE 工作真用,真出问题立刻反馈 |
| 多平台兼容(尤其 Windows) | 中 | Phase 1-3 macOS+Linux 主用,Windows 在 Phase 4 集中 fix |

## 10. 不做(本战略)

| 能力 | 理由 |
|---|---|
| Strangler Fig 渐进迁移 | doc/14 v0.1 复盘后否决,副业不需要 |
| 整体 Go 替代(而非 Rust) | 团队 Rust+WASM 经验不浪费 |
| 自研 WASM runtime | 用 wasmtime |
| 自研列存格式 | 用 Arrow |
| Pure-Rust UI(Dioxus / Yew / Leptos) | 生态不如 React 成熟,前端代码可复用 90% |
| 改前端语言(React → Vue / Svelte) | React 18 + AntD 已稳,不动 |
| SaaS / 多租户 / 账号体系 | 后期通过 engine-cli + Web shell 扩,**不是 v1.0 目标** |
| 移动端 / Web 浏览器版本 | Tauri 桌面专属 |
| 自定义 K8s operator | 引擎是数据消费者,不是 K8s controller |

## 11. 成功度量(T+12mo)

主观但具体:

| 度量 | 目标 |
|---|---|
| Tauri 桌面 app 跨平台构建 | macOS + Linux + Windows 三平台 binary |
| Rust 核心引擎 LOC | 18-28k(对齐 Python 规模,部分模块更紧凑) |
| WASM 插件数 | ≥ 8 个(6 connector + 2 rule + N 自定义 handler) |
| 自用频率 | 主业 SRE 工作每周打开 ≥ 3 次 |
| 公开 blog 系列 | ≥ 4 篇 |
| GitHub stars | ≥ 150(弱信号,不强求) |
| 个人技术深度 | Rust + WASM + Tauri + SRE,**这是最重要的隐性 ROI** |

## 12. 立即下一步(本周末)

1. ✅ doc/14 v0.2 / doc/16 v0.2 / doc/17 / doc/15 更新
2. [ ] `git mv backend/ reference/`,加 DO NOT DEPLOY README
3. [ ] `git mv frontend/ desktop/`(空 Tauri 壳留待 Phase 1 填)
4. [ ] 顶层 `Cargo.toml` workspace 骨架(engine/* + modules/* + desktop/src-tauri)
5. [ ] GitHub project board + 12 个月 milestone 公开

## 13. 相关文档

- 起点:[`13-story-unknown-dep-stripe.md`](./13-story-unknown-dep-stripe.md)
- 数据契约:[`15-data-contract-spec.md`](./15-data-contract-spec.md)
- 仓库布局:[`16-repo-and-codebase-layout.md`](./16-repo-and-codebase-layout.md)
- Tauri 架构:[`17-tauri-desktop-architecture.md`](./17-tauri-desktop-architecture.md)
- 上下文:[`10-product-gap-analysis.md`](./10-product-gap-analysis.md) / [`11-PRD-005-...`](./11-PRD-005-universal-topology-service.md) / [`12-PRD-006-...`](./12-PRD-006-code-repo-source.md)
- 导航:[`00-README.md`](./00-README.md)

---

## 附录 A — 考虑过但放弃的路径

### A.1 Strangler Fig(原 v0.1 方案)

老 Python 跑业务,新 Rust 模块逐个替换,gRPC/REST 桥接,18-24 个月双语言并存。

**否决理由**:Strangler 的核心好处(零停机持续交付)对副业不成立 — 没有 7×24 跑的客户。但代价照收:

- 桥接代码 ~3 mo
- 双语言 contract YAML ~3 mo
- 4 套 toolchain 维护 ~2 mo
- Context switch 隐形税 ~5 mo
- **总浪费 ~13 mo,换不需要的能力**

副业场景下 Strangler 是过设计。

### A.2 Clean-sheet 完全抛弃 Python

不看老代码,从零设计。

**否决理由**:472 测试 + 12k LOC 里 embedded 的边界 case(Neo4j Path 三类异常 / BFS 深度 4 不是 5 / diff_summary JSON 编码 / verify 防递归)再走一遍 80% 都会踩。Chesterton's Fence。

Supervised Rewrite 保留 Python 作 oracle,既享重写的清洁,又保留知识。

### A.3 走 SaaS Web 默认路径(原 v0.1 方案)

FastAPI + Web 前端 + Docker Compose / k8s 部署。

**否决理由**:

- 副业不应该背 SaaS 包袱(账号 / 计费 / 多租户 / 7×24 运维)
- 个人 SRE 工具的最佳形态参考 k9s / Lens — 都是桌面
- Tauri 单二进制 + 本地数据,**摩擦最小**
- 想做 SaaS,engine-cli 已经在,后期加 web shell

---

**版本**:v0.2.0 — 2026-06-23 Supervised Rewrite + Tauri 决策定稿。v0.1 Strangler Fig 路径见 附录 A.1。
