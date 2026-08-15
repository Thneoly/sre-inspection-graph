# 变更与告警的时间线:传播 BFS、YAML diff 去噪与 poll-diff 自动录入

> 故障排查的第一个问题永远是「最近改了什么」。这篇讲变更追踪:怎么记录变更、怎么算影响面、怎么把 K8s 的 YAML diff 从噪声海里剥出信号,以及怎么在桌面架构的限制下把录入自动化。

## 变更为什么难

「改了什么」在各处都有记录 —— Git 有 commit,Argo 有 sync 记录,K8s 有 annotation —— 但它们回答不了 SRE 的真问题:

- 这次改的是 `resourceVersion: 52731 → 52732`,**人话是什么**?
- 这个 ConfigMap 改了,**会影响哪些 Pod / 组件 / 应用**?
- 告警 14:32 响的,**14:30 那次变更是不是元凶**?

所以变更追踪要做的三件事:结构化的变更事件、沿拓扑算**影响面**、和告警做**时间窗关联**。

## 事件模型:一条 ChangeEvent 记什么

18 个字段里最关键的几个:`change_type`(configmap_updated / secret_rotated / deployment_rolled / image_pushed)、`target_resource_id`(拓扑里的节点)、`propagated_to`(影响面,后面算)、`yaml_diff`(人话的差异)、`severity_estimate`。severity 有个简单的统计估计:同资源一小时窗口内变更 ≥10 次升 high、≥5 次升 medium —— 配合「过频变更」标签,把「这玩意一天改八回」这种慢性病暴露出来。

## 影响面:反向 BFS + 边白名单

「ConfigMap 改了影响谁」在图上是个遍历问题:从 ConfigMap 出发,沿**反向**的引用边(谁 USES 我)向上游找。但不是所有边都该算 —— 白名单只有 8 种具备「依赖传播」语义的:

```
USES / CONTAINS / DEPLOYED_AS / BELONGS_TO /
RUNS / SCHEDULED_ON / EXPOSES / ROUTES_TO
```

刻意排除的例子:`USES_IMAGE`(容器用某镜像)不进传播白名单 —— 推镜像不等于改了这个服务的运行状态。**影响面的可信度来自克制**:宁可少报,不要把不相干的资源拉进来制造恐慌。

同样做成纯函数:吃 `&Topology` 反向 BFS,depth 限 4。和上篇的 dry-run 是同一个原语的两种参数化 —— 图遍历在这个系统里是通用底座。

## YAML diff:一半的功力在去噪

直接 diff 两个 K8s 资源的 YAML,你会得到一屏 nobody-cares:

```diff
-  resourceVersion: "52731"
+  resourceVersion: "52732"
-  uid: 8f2a...
   managedFields:
-    {manager: kube-controller-manager, time: ...}
```

所以我维护一张**噪声字段表** —— 10 个字段在任何变更里都只代表「K8s 在自转」:`managedFields` / `resourceVersion` / `uid` / `creationTimestamp` / `generation` / `selfLink` / `etag` / `last-applied-configuration` / `annotations` / `managedVersion`,递归剥掉之后再 diff。

更进一步:**变更检测只盯信号字段**。Deployment 的信号字段是 `current_revision`(rollout 计数)、`images`、`replicas_desired` —— 上一轮这些值与这一轮相等,就当没变。于是 rollout restart 之后,记录下来的 diff 是这样的:

```diff
current_revision: 1 → 2
```

一行,精确,就是这次变更的全部人话。这也顺带修掉一个误报陷阱:如果拿 `ready < desired` 当变更信号,一次普通的滚动过程会被误判成「变更了好几次」。

信号字段表在代码里就是这样一张映射(`engine-changes/src/watch.rs`,真实代码):

```rust
fn signal_keys(resource_type: &str) -> Option<&'static [&'static str]> {
    match resource_type {
        "ConfigMap" | "Secret" => Some(&["data_keys"]),
        "Deployment" => Some(&["current_revision", "images",
                               "replicas_desired", "replicas_ready"]),
        _ => None,
    }
}
```

一个实现细节:diff 的 YAML 输出是我**自己写的确定性发射器**(按键名排序、固定 block-style),没有引入 serde_yaml —— 序列化库的格式随版本漂移的话,「字符串相等的 diff 基准」就烂了。确定性在这里不是洁癖,是正确性依赖。

## 自动录入:桌面架构下的两条路

Webhook 是变更录入的经典答案,但我的架构刻意不起 HTTP server(见系列第一篇)—— 没有入站连接。于是两条替代路:

**路一:poll-diff。**后台每轮同步把新拓扑和上一轮做 diff,`detect_changes(current, next)` 是纯函数,只看 ConfigMap / Secret / Deployment 的信号字段。有个细节必须处理:**首次同步抑制** —— 程序刚启动拿到全量拓扑,如果不过滤,会把「历史上已经发生的所有变更」当成本轮新变更灌进去(重启一次,时间线爆炸)。用一个标志位:第一轮只建基线,从第二轮开始才检测。

**路二:事件流 connector。**K8s Events 本身就是变更信号源(`ScalingReplicaSet` → deployment_rolled)。做一个 WASM connector 轮询事件接口,这里出现一个有意思的模式 —— **有状态 guest**:它第一轮只把事件 UID 存进 baseline 不上报,之后每轮只报新 UID。状态放在 WASM 模块的 `thread_local` 里,跨轮次存活。这打破了「connector 应该无状态、重扫幂等」的默认假设,但换来的是事件语义的精确去重 —— 是个值得记录的权衡。

两条路并存:poll-diff 抓「字段级变化」,事件流抓「K8s 认为值得记录的事件」,互补。

## 和告警、和恢复的闭环

- **告警关联**:告警的 `resource_ref` 落在变更的影响面(`{target} ∪ propagated_to`)且时间在窗内 → 关联上。双向可查:从变更查「它解释了哪些告警」,从告警查「窗口内有哪些变更」。注意这是**相关不是因果** —— 时间窗关联只能给线索,结论留给人。
- **恢复建议**:从变更事件可以一键拉起[上一篇](./05-recovery-action-engine.md)的动作推荐 —— `deployment_rolled` 推荐 `rollback_deployment`,`secret_rotated` 推荐 `refresh_secret`。变更 → 定位 → 恢复,一条线。

## 小结

- 变更追踪的价值三件套:**结构化事件 + 拓扑影响面 + 告警时间窗**,缺一个都只是日志;
- YAML diff 一半功力在去噪:噪声字段表 + 只盯信号字段,把一屏 `resourceVersion` 换成一行人话;
- 传播边白名单要克制 —— 影响面的可信度来自「宁少勿滥」;
- 没有 webhook 的架构下,poll-diff(带首同步抑制)+ 事件流 connector(有状态 guest)是可行的双路;
- 确定性序列化是字符串 diff 的正确性前提,值得自己写发射器。

完整实现(`engine/crates/engine-changes/`):**https://github.com/Thneoly/sre-inspection-graph**

> 系列上一篇:[让恢复动作敢按下去](./05-recovery-action-engine.md) ｜ 下一篇:[一个 subgraph 原语,六个巡检视图](./07-subgraph-views.md)
