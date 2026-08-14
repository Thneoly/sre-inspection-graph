# 同一资源、不同 ID:多源拓扑的 Identity Resolution

> 图谱平台做到后面,最难的不是「把数据画出来」,是**同一个东西被两个数据源叫了两个名字**的时候,你怎么知道它们是一个。这篇讲我的解法(correlation-key 合并),以及一个可能比解法更值钱的工程判断:这个功能我推迟了两个 Phase 才做。

## 问题:一张图里的「幽灵双胞胎」

我的平台有多个数据源。K8s connector 看到的容器镜像是:

```
image:vm-cluster:otel-demo:ghcr.io/open-telemetry/demo:1.11.0-cartservice
```

而 code-repo connector 扫描本地代码仓的 Dockerfile,看到的是:

```
image-ref:ghcr.io/open-telemetry/demo:1.11.0-cartservice
```

**同一个镜像,两个 ID。**如果放任不管,图上会出现两个节点,然后:

- 代码仓产的 `BUILDS` 边(repo → 镜像)指向一个**孤立的节点**,和运行时的世界永远连不起来;
- 「这个镜像出了漏洞,哪些服务在用」这类查询只覆盖运行时侧,**代码侧的证据链是断的**;
- 更糟的是这种割裂是静默的 —— 图看起来正常,只是「少了些本来该有的连接」。

这正是各家公司做统一拓扑服务(以及 CMDB)时的经典难题:**身份(identity)是一切关联的前提,而身份恰恰是最难对齐的。**

## 解法:correlation-key 合并

思路一句话:**每个数据源在描述资源时,除了自己的 ID,再挂一个(或多个)跨源通用的关联键;合并器把共享关联键的节点合成一个。**

镜像的天然关联键是镜像引用本身(规范化后):

```
k8s 侧节点 attrs:
  correlation_keys: ["image-ref:ghcr.io/open-telemetry/demo:1.11.0-cartservice"]

code-repo 侧节点 attrs:
  correlation_keys: ["image-ref:ghcr.io/open-telemetry/demo:1.11.0-cartservice"]
                                  ↑ 同一个 key
```

注意一个细节:镜像引用要先**规范化**(去掉 digest 后缀、补全 `:latest`、剥掉默认 registry 前缀),否则 `demo:1.11.0-cartservice` 和 `ghcr.io/open-telemetry/demo:1.11.0-cartservice@sha256:...` 又是「两个」。规范化函数放在两侧共享的 SDK 里,而不是各写各的 —— 不然合并器会在规范化差异上再裂一次。

### 合并算法

合并器是 resolve 管线里的一个预处理 pass,输入一整批 Fact,输出改写后的批次:

```
1. 收集:扫所有节点 Fact 的 correlation_keys
2. 聚簇:把「共享任一 key」的节点连通(BFS,支持传递合并:
        A—key1—B—key2—C ⇒ A/B/C 同簇)
3. 选 winner:每簇挑一个 canonical 节点
        优先级 = source 优先级(k8s=10 > code-repo=5)
        平局 → resource_id 字典序最小          ← 保证决定性
4. 合并 attrs:winner 的属性为准,loser 补 winner 缺的 key
        用 BTreeMap-backed JSON → to_string()
        产出 canonical 有序串                  ← 又是决定性
5. remap:所有指向 loser 的边端点/父指针改指 winner
        loser 节点丢弃
```

拿真实数据跑:K8s 的镜像节点赢了(运行时源 > 声明源,这符合直觉 —— 运行时观察到的更可信),code-repo 的 `BUILDS` 边 remap 到 K8s 镜像节点上。于是图上出现了这条横跨两个世界的链:

```
CodeRepo ──BUILDS──→ ContainerImage ←──USES_IMAGE── Container
 (声明:谁构建它)      (合并后的唯一节点)              (运行时:谁在跑它)
                              ↑
                        Pod ──┘ (SCHEDULED_ON Node)
```

「哪个仓库构建了这个镜像、哪些 Pod 正在跑它、跑在哪台 Node 上」一跳全通。这正是这张图存在的意义。

## 决定性是 load-bearing 的,不是洁癖

这个算法有个不显眼但致命的要求:**同样的输入(不管什么顺序),必须产出字节级相同的输出。**

为什么?因为下游的增量同步靠**字符串相等**判断「这个节点变没变」:

```
diff(current_topology, next_topology)
  节点变了没 = attributes_json 字符串相等?
```

如果合并在不同 sync 顺序下产出属性顺序不同的 JSON(哪怕内容一样),diff 会把所有合并节点判为「变了」,每次 sync 都全量重写 —— 图谱的增量维护就废了。所以 winner 平局要字典序、属性要 canonical 排序,并且专门有一个单测:**打乱输入顺序,断言节点集合一致 + 合并后 attrs 字节一致**。

这种「不变量没守住就静默劣化」的坑,最好在写算法之前就想清楚,而不是等症状出现再倒查。

## 零 schema 改动:把线索藏在 attributes 里

实现上还有个取舍值得说:correlation_keys 我**没有**做成 Fact 或节点的新字段,而是塞在 `attributes_json` 里。

这意味着:WIT 接口、Arrow Schema、SQLite 表、Parquet 文件,**全部一行没改**。合并线索作为一种「随数据流动的注记」存在,不关心它的模块根本看不见它。

代价是类型不安全(key 写错了就是合不上,而不是编译错误)—— 但对比「为一种合并线索动四层契约」,这笔账在当前阶段显然划算。等 v2 需要给 key 加 provenance / confidence 时,再考虑提升为一等字段也不迟。

## 比解法更值钱的判断:我推迟了它两个 Phase

坦白讲,这个功能我在做技术债清理的 Phase 6 时**就能写**:造几个合成节点、给它们挂上相同的 key、演示合并 —— 演示效果会很漂亮。

我没做。因为**合成数据造出来的合并冲突是假的**:没有真实的两个数据源对同一个资源各自描述,你就不知道 key 会怎么脏(规范化差异、大小写、digest 变体)、winner 优先级该怎么排、边界情况长什么样。整套仲裁逻辑会对着不存在的问题空转 —— 代码写出来了,但每一条规则都是猜的。

它一直推迟到 code-repo connector 落地,「代码仓的 BUILDS 边」和「K8s 的部署镜像」成了**一对真实的、互相不知道对方存在的描述**,合并才有了非做不可的理由和真实的问题形状。落地那天,两侧对真实部署的镜像发出同一个规范化 key,一次跑通。

这件事给我的教训比算法本身大:**知道什么时候 NOT to build,和知道怎么 build 一样重要。** 合成数据能验证代码路径,验证不了问题本身 —— 而架构决策的质量,取决于你对真问题的理解。

## 小结

- 多源图谱的核心难题是**身份对齐**;correlation-key + 图聚类 + 明确的 winner 仲裁,是够用且可解释的最小方案;
- **决定性**(顺序无关、字节稳定)是合并器的 load-bearing 不变量,必须显式设计 + 单测钉死,否则增量 diff 静默劣化;
- 能藏在现有契约里表达的语义,不要急着升为一等字段 —— schema 改动的成本是四层放大的;
- 一个功能值不值得现在做,看它有没有**真实数据形状的问题**在等,而不是看它能不能被演示。

完整实现(`engine-identity/src/correlation.rs`,8 个单测 + resolve 级集成测试):**https://github.com/Thneoly/sre-inspection-graph**

> 系列上一篇:[canonical Fact 与三层数据契约](./03-canonical-fact-data-contract.md) ｜ 回到系列开篇:[一个人从 Rust 内核做到 React 前端](./01-fullstack-sre-graph-tool.md)
