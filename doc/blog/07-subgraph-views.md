# 一个 subgraph 原语,六个巡检视图 —— 顺便聊聊词表漂移

> 巡检平台的最后一公里是视图:SRE 打开工具想问的六个问题,每个听起来都要单独开发。这篇讲我怎么发现六个视图其实是**一个图遍历原语的六组参数**,以及在这个过程中踩到的、比视图本身更值得写下来的坑:词表漂移。

## 先看六个问题

| 视图 | SRE 在问什么 |
|---|---|
| 应用拓扑 | 全局长什么样 |
| 节点影响 | 这台 Node 挂了,爆炸半径多大? |
| 配置影响 | 这个 Secret / ConfigMap 变了,谁受影响? |
| 访问链路 | 流量从应用入口怎么流到 Pod? |
| 镜像风险 | 这个镜像被哪些服务在用? |
| 告警聚合 | firing 的告警各自挂在哪、周围是什么? |

前五个听起来是五种查询,写出来会发现它们共享同一个形状:**从某个起点出发,沿某几类边,朝某个方向,走有限深度,把够得着的子图拿出来。**唯一变化的是四个参数:

```
subgraph(topology, start, max_depth, allowed_edges, direction)
```

实现就是 BFS 收集可达节点 ID,然后过滤节点 + 过滤边(两端都在子图内、且边类型在白名单)—— 返回 **induced subgraph**(子图内部的边全保留,不只有 BFS 走过的边)。逐字摘录(`engine-identity/src/views.rs`;`// …` 开头行是我标的省略,其余与源文件完全一致):

```rust
pub fn subgraph(
    topo: &Topology,
    start: &str,
    max_depth: usize,
    allowed: &[&str],
    dir: TraversalDir,
) -> Topology {
    // start 不在拓扑 -> 空(起点缺失返空图)
    if !topo.nodes.iter().any(|n| n.resource_id == start) {
        return Topology::default();
    }

    let allowed: HashSet<&str> = allowed.iter().copied().collect();

    // BFS 收集可达节点 ID
    // …(frontier 初始化与起点入队,略)
    while let Some((node, depth)) = frontier.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for e in &topo.edges {
            if !allowed.contains(e.edge_type.as_str()) {
                continue;
            }
            let next = match dir {
                TraversalDir::Forward => {
                    if e.source == node {
                        Some(e.target.as_str())
                    } else {
                        None
                    }
                }
                TraversalDir::Reverse => {
                    if e.target == node {
                        Some(e.source.as_str())
                    } else {
                        None
                    }
                }
                // …(Both 分支同构:任一端命中取另一端,略)
            };
            // …(visited 去重后入队,略)
        }
    }
    // …(按 visited 过滤 nodes + edges,返回 induced subgraph,略)
}
```

~35 行,8 个单测。然后五个视图是五张参数表:

```
节点影响:  start 类型 = Node,   方向 = Reverse,  深度 4   (谁调度/运行在这台上)
配置影响:  start 类型 = Secret|ConfigMap, Reverse, 深度 4  (谁在用这个配置)
访问链路:  start 类型 = Application, Both,     深度 5   (上下游都看)
镜像风险:  start 类型 = ContainerImage, Reverse, 深度 4  (谁在跑这个镜像)
```

方向是关键参数:「影响面」类问题永远是**反向** —— 变更在 ConfigMap 上,答案在引用它的 Pod 上,边是从 Pod 指向 ConfigMap 的,所以要逆着边走。

白名单直接从各视图的语义来(访问链路只沿 CALLS / EXPOSES / ROUTES_TO 这类「流量路径」边)。一个务实细节:白名单里有几种边类型我的 connector 目前还不产 —— **不匹配即无害**,等以后有数据源产出它们,视图自动变完整。参数表比 switch-case 友好就在这。

## 第六个视图为什么不是参数

**告警聚合**长得像,但起点根本不是拓扑节点 —— 是告警注册表里 firing 的告警。它的做法:查询时把每条 firing 告警**合成**一个图节点 + 一条 `FIRED_ON` 边指向它的 `resource_ref`(如果在拓扑里),再从那个资源用同一个 `subgraph` 原语向两侧展开邻域。

这是个「查询时 join」:告警是 L3 动态观测,拓扑是 L2 结构,两边各自演进,合成发生在读的时候。不为了渲染把告警物化进拓扑表 —— 那会让「告警恢复」变成一次图变更,不值得。

一个去重细节:同一个资源上挂着三条告警,邻域只展开一次 —— 不然三倍节点把画面糊掉。

## 词表漂移:比视图本身更值钱的教训

视图上线后第一次真集群验证,「节点影响」的起点选择器是**空的**。查了半天,原因很朴素也很扎心:

- 视图代码(照着设计文档)查询 `KubernetesNode` 类型的节点作为起点候选;
- 而我的 K8s connector 实际产出的类型叫 `Node`。

两个词表,一个在设计文档里,一个在 connector 代码里,**没有任何机制保证它们一致**。选择器查了个不存在的类型,空 —— 还算走运;更险的情形是静默地查到错的数据。

这类问题的阴险之处:类型系统帮不了你 —— 两边都是合法字符串;测试也未必帮你 —— 单元测试用的是各自一边自己造的 fixture,自己跟自己当然一致。**漂移只发生在两边的接缝上,只有跨接缝的测试能抓到。**

修了三层:

1. **中央注册表**:`resource_type` / `edge_type` 的常量表 + `is_known()` 校验,host 侧所有生产消费点(视图白名单、传播白名单、恢复动作规则)统一引用,不再各写各的字符串;
2. **回归守卫**:注册表测试里有一条 `is_known("KubernetesNode") == false` —— 把这个错误拼写永久钉死,谁加回去谁挂测试;
3. **realistic-fixture 集成测试**:用「真实 connector 产出形状」的 fixture 跑视图断言,保证接缝两边对得上 —— 单元测试管逻辑,集成测试管词汇。

诚实说,缝没有完全焊死:WASM connector 是独立 workspace,不依赖 host 的注册表 crate,guest 产出的字符串和 host 的注册表之间仍是约定而非类型。目前靠 fixture 测试 + 一个 headless 巡检工具(对真库跑六个视图做冒烟)把关;彻底的解法是未来从 WIT 契约生成两边的词表 —— 单一真源,接缝消失。

> 这条的普适性超出这个项目:**任何「两边各自定义、靠字符串对接」的系统都有这个词表漂移问题** —— 前后端的枚举、微服务间的错误码、数据库和 ORM 的类型名。解法永远是同一个:单一真源 + 跨接缝测试,缺一个都会在某个深夜爆出来。

## 小结

- 多个相似视图先找**原语**:六个视图 = 一个 subgraph × 五张参数表 + 一个合成的特例,代码量和 bug 面都少一个数量级;
- 「影响面」类查询的方向永远是反向遍历,方向参数比深度参数更容易搞错;
- 告警聚合用**查询时 join**(合成节点 + 即席邻域),不把动态观测物化进结构表;
- **词表漂移**是接缝病:类型系统和单元测试都抓不到,只有单一注册表 + 跨接缝的 realistic fixture 能防 —— 以及,把错误的拼写写成一条失败的测试,让它永远死掉。

实现:`engine-identity/src/views.rs`(subgraph 原语)+ `desktop commands/views.rs`(参数表)/**https://github.com/Thneoly/sre-inspection-graph**

> 系列上一篇:[变更与告警的时间线](./06-change-tracking-timeline.md) ｜ 下一篇:[给观测工具自己做观测](./08-observability-in-proportion.md)
