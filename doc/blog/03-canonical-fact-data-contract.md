# canonical Fact 与三层数据契约:一个平台可扩展的支点

> 我在做 SRE 图谱工具时做过最重要的一个决定,不是选 Rust 也不是选 Tauri,而是:**所有数据源,不管原本长什么样,进来之后都变成同一种东西**。这篇讲这个决定怎么落地,以及它后来怎么反复救了我。

## 先算一笔耦合账

设想最直觉的做法:K8s connector 拉到的是 K8s 的 JSON,Jaeger connector 拉到的是 trace,代码仓扫描到的是依赖清单 —— 各自直接往图里写节点和边。

看起来最省事,但下游不止一个「图」:

```
                  ┌→ 存储层(SQLite / Parquet)
connector 们的产出 ├→ identity resolution(多源合并)
                  ├→ graph build(去重/派生边/悬空过滤)
                  └→ UI 的 GraphResponse
```

每种数据源 × 每个下游 = 一条适配路径。6 个 connector、4 个下游,就是 24 条要维护的路;加第 7 个 connector,要同时改 4 个下游。**耦合数 = N × M,这是平台失控的经典起点。**

## 决定:一个 canonical 中间态

所有 connector 的产出统一压平成一种 7 字段的结构,我叫它 **Fact**:

```rust
pub struct Fact {
    pub id: String,               // 幂等键(同 id 重复 sync 覆盖)
    pub kind: String,             // topology-node / topology-edge / metric / change / alert
    pub source: String,           // 谁产的:k8s / jaeger / code-repo / ...
    pub resource_id: String,      // 资源身份(节点 id / edge:{type}:{src}->{tgt})
    pub resource_type: String,    // Pod / Deployment / ContainerImage / Edge ...
    pub timestamp: u64,           // 观测时间
    pub attributes_json: String,  // 全部属性,一个 JSON 字符串
}
```

规则只有一条:**所有下游只认 Fact**。存储层存 Fact、identity 消费 Fact、graph 从 Fact 建、UI 拿到的也是 Fact 的投影。K8s JSON 的形状、Jaeger trace 的结构,到了内核边界就消失了。

再把这条契约焊死在类型层:`engine-core` 里有一个 `fact_schema()`,返回这张 7 列表的 **Arrow Schema**。`FactBatch` 能零拷贝转成 Arrow `RecordBatch`(Parquet 归档走这条);SQLite 落库的列也由同一个 schema 定义 —— 同一个 schema 管三个去处,不存在「存的时候一列、读的时候另一列」。

```
Fact ──→ RecordBatch ──→ Parquet(append-only 归档,按日期分区)
  └──→ SQLite(行级 upsert,最新态 + 物化拓扑)
```

## 三层契约:每个边界一个协议

Fact 解决「数据长什么样」,但一个系统里还有别的边界。我给每个边界都定了一个**唯一的**协议,并且刻意不搞第二套:

| 边界 | 协议 | 为什么是它 |
|---|---|---|
| WASM 插件 ↔ 宿主 | **WIT**(Component Model) | 跨语言、类型安全;宿主和插件各自 bindgen,不共享代码 |
| 前端 ↔ Rust 后端 | **Tauri commands**(进程内 JSON IPC) | 不起 HTTP server,无序列化往返;webview 是沙箱只能调白名单命令 |
| 引擎内部 | **Arrow + SQLite + Parquet** | Arrow 是批传输的规范内存格式,Parquet 是归档的事实标准 |

注意每一层都**只有一个**协议。反过来说,我也明确记了「反模式」清单,写代码前对着看:

- ❌ 在桌面进程里再起一个 HTTP server —— 已经有进程内 IPC,REST 是画蛇添足;
- ❌ 让 Arrow `RecordBatch` 跨 IPC 到前端 —— 批数据留在 Rust 侧,前端只拿查询结果的 JSON 投影;
- ❌ WASM 插件直接 syscall —— 一切外部访问经宿主 capability(上一篇的主题)。

## 这套设计怎么反复救我

最有说服力的不是道理,是后来的账。

**加 connector 变成了「纯增量」**。整个项目先后加了 5 个 connector —— prometheus(PromQL)、jaeger(聚合跨服务 span 成 CALLS 边)、k8s-events(K8s 事件流)、flagd(特性开关 diff)、code-repo(扫描本地仓库)。**每一个都是只新增一个 WASM 模块 + 一条 manifest 配置,内核和全部下游零改动。** 拓扑合并、去重、悬空过滤、视图、恢复引擎对新数据源一无所知,但自动生效 —— 因为它们只认 Fact。

**新增一种 Fact 语义只动一处**。后来要支持「变更事件」(ChangeEvent),做法是引入 `kind="change"` 的 Fact,在 sync 管线里加一个路由:遇到这种 kind 就转成变更记录而不是拓扑。下游的图逻辑完全没动。

**契约测试有了明确的锚**。因为 schema 唯一,每个领域函数的测试就是「喂一组 Fact fixture + 断言输出」,没有各数据源的形状要 mock。517 个测试里相当一部分就是这么写的,便宜且稳。

## 一些实操细节

**幂等靠 `id`,最新态靠时间戳**。同一个资源反复 sync,新 Fact 覆盖旧 Fact(SQLite upsert);「当前拓扑」查询取每个 resource_id 最新的一条。这比「每次全量重建」便宜,也让离线重放成为可能。

**`kind` 是自由字符串,不做 enum**。这是有意的:K8s 的资源类型和边类型用了中央注册表(常量表 + `is_known` 校验,防词表漂移),但 Fact 的 kind 保持开放 —— 因为我预见到会有 `unknown-dep` 这类未来的语义,不想为加一种 kind 改 schema。事实证明这个口子留对了。

**attributes 用 JSON 字符串而不是强类型字段**。不同资源的属性差异巨大(Pod 有 phase,镜像有 digest,仓库有 git url),压平成 JSON 让 Fact 保持 7 列不变;后来的 correlation-key 合并甚至复用了这个字段装合并线索,**零 schema 改动**完成了本来要动表结构的功能(这是下一篇的故事)。

## 小结

- 平台的可扩展性 = **N + M**,而不是 N × M —— 办法是让所有变化方向汇到一个 canonical 中间态;
- 契约要**焊在类型层**(Arrow Schema / WIT / command 签名),文档契约会漂移,类型契约不会;
- 每个边界**只留一个协议**,并显式维护反模式清单 —— 架构纪律的一半是知道不做什么;
- 开放字段(kind 自由、attrs JSON)不是不严谨,是把「确定会变的部分」和「不该变的部分」分开。

仓库与完整契约文档(`doc/15`):**https://github.com/Thneoly/sre-inspection-graph**

> 系列上一篇:[WASM capability 沙箱](./02-wasm-capability-sandbox.md) ｜ 下一篇:[同一资源、不同 ID:Identity Resolution](./04-identity-resolution.md)
