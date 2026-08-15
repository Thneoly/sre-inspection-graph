# 让恢复动作敢按下去:dry-run、审批门、自动回滚与 mutable twin

> SRE 最怕的往往不是发现故障,是把手悬在「重启 / 回滚 / 扩容」按钮上的那几秒 —— 影响面说不清,按下去了回不来怎么办。这篇讲我的恢复动作引擎怎么把「敢按」这件事工程化。

## 设计目标

一个恢复动作引擎要回答四个问题:

1. **按下去之前**:会影响到谁?(预演)
2. **谁有权按**?(审批)
3. **按下去之后**:怎么知道它生效了?(验证)
4. **搞砸了**:**怎么反悔**,而且不用人再审一遍?(回滚)

我把这四问做成一条管线,每个动作走完全程:

```
pending → dry_run_ok → [awaiting_approval] → executing → succeeded
                                          ↘ verify → verify_failed?
                                                        ↓ 是 + 有回滚动作
                                                   自动反向回滚 → rolled_back
```

## dry-run:把「爆炸半径」算成一张表

每个动作带一张**传播规则表**:沿哪几类边、什么方向、多深、命中什么类型的资源、影响多大。dry-run 就是拿这张表在当前拓扑上跑一遍 BFS:

```
scale_deployment(target: Deployment replicas +1)
  ├→ Deployment 本体          impact: high
  ├→ ─CONTAINS→ Pod ×3        impact: medium   "滚动重启所有实例"
  └→ ─SCHEDULED_ON→ Node ×1   impact: low      "调度压力略增"
affected_count: 5 · estimated_sla_impact: medium
```

关键实现选择:**dry-run 是纯函数**,吃 `&Topology` 不碰任何 I/O。这意味着「预演」可以在任何时刻对任何拓扑快照跑 —— 包括测试里的合成拓扑,也可以在审批门里再跑一遍确认这半小时拓扑没变。

8 个动作(scale / restart_pod / rollback_deployment / refresh_secret / drain_node / kill_query / restart_service / clear_cache)各自有规则表;`drain_node` 的深度最大(节点上所有 Pod + 它们的主人链),`scale` 最浅。

## 审批门:风险决定要不要人点头

规则简单到一张表:**low 同步执行;medium / high 进 `awaiting_approval`,等人确认。**

```
low:     scale_deployment, kill_query, restart_service
medium:  restart_pod, refresh_secret, clear_cache
high:    rollback_deployment, drain_node
```

一个诚实的取舍:原设计里有审批人团队推导、24 小时 TTL 过期、多人 approve-reject 流。我把它砍成了**桌面单机确认门** —— 这是一个单机工具,没有多人协作场景,做一套多角色审批是给演示造复杂度。风险等级 → 状态的映射保留,因为那是真正影响交互安全的部分。

**回滚跳过二次审批**是刻意的:原始动作已经被人审过了,反向操作是「撤销已审的决定」,不是新的风险决定。再让人审一遍回滚,只会让紧急时刻多一道卡壳。

## mutable twin:让「验证」和「正确地反悔」成为可能

这是整个引擎里我最喜欢的设计。问题这样开始:动作执行完,怎么验证它生效了?而且 —— 如果要回滚,**回滚必须基于「动作执行后的状态」做正确的反转**,而不是拿动作前的参数瞎倒。

我的做法:执行时把目标拓扑 clone 一份**孪生(mutated twin)**,handler 在孪生上**写回动作生效的字段**:

```
scale +1 在孪生上写:
  desired_replicas: 3 → 4
  available_replicas: 3 → 4

verifier 读孪生验谓词:
  new_replicas == 4 ?  ✓ passed

若 verify 失败 → 回滚读的是孪生的 post-action 状态:
  new_replicas=4 → 反向 delta −1 → 回到 3(而不是拿动作前的 3 再减一次)
```

verifier 每个动作一个,读孪生上 handler 写的字段验谓词。两个无可观测副作用的动作(`kill_query` / `clear_cache`——mock 世界里没有慢查询计数器)诚实地返回 `not_supported`(passed=true),不装模作样地「验证通过」。

**自动回滚**串起来:`verify_failed` 且动作配了 `rollback_action_id` → 自动执行反向动作,带一个 marker 防递归(回滚的回滚不会无限套娃);没配 rollback 的动作只告警不自动反悔(比如 `drain_node`,反悔它的语义是重新调度,不是简单反向)。

孪生是 clone 出来的,**不写回**物化拓扑 —— 真实世界的拓扑只由数据源同步更新,mock 的动作效果留在孪生里。这样「模拟演练」永远不会污染「真实状态」。

## 动作链:多步恢复只审一次

真实的恢复常常是多步:「换 Secret → 滚动重启 → 观察三分钟」。链模板把步骤声明出来,失败策略三选一:

- **Stop**:停在人这边,等人看;
- **RollbackAll**:反向回滚已完成的步骤(逆序);
- **Continue**:记录失败,继续走完。

链级审批语义:**任一步是 medium/high,整链审一次** —— 不是每步卡一道门。这在「安全」和「紧急时刻可用」之间取了实用的一点。

## 一次完整的排查:real 模式第一天,verifier 全军覆没

这是整个引擎开发里最值得复盘的一战。接真实 K8s handler(WASM handler 经 `http-write` 真改集群)后的第一次端到端验证:scale +1 执行成功、集群副本数真的变了 —— 然后 **8 个 verifier 全部判 fail,自动回滚被误触发,把刚做的扩容又缩了回去**。

**第一层:症状。**verifier 读的是执行后节点的 `attributes_json`,翻执行记录发现:handler 返回的 result 里根本没有 verifier 期望的字段名(`new_replicas`)。那为什么 mock 模式从来没暴露过?—— mock handler 和 verifier 是我同期写的,字段名**恰好互相咬合**;而 WASM handler 是另一端独立实现的,它返回的是自己的字段名(`desired_replicas`)。两个独立实现要对齐到一份**谁也没写下来的隐式契约**上,不咬合是必然,咬合才是巧合。

**第二层:更深的雷。**修字段名时发现真正危险的问题:宿主若把 WASM handler 返回的 attrs **整体替换**到节点上,会把 connector 之前写入的字段(`cluster` / `name` / `replicas_desired`…)全部擦掉 —— 图谱节点从此残缺,残缺还会被物化,下一轮 diff 把它放大成「全图变更」。修法是显式的 **overlay 合并**(真实代码):

```rust
// engine-wasm/src/handler_executor.rs
fn merge_values(base: Value, overlay: Value) -> Value {
    let mut m = match base { Value::Object(m) => m, _ => Map::new() };
    if let Value::Object(o) = overlay {
        for (k, v) in o { m.insert(k, v); }   // 动作字段覆盖,connector 字段保留
    }
    Value::Object(m)
}
```

**第三层:把隐式契约写显。**verifier 期望的字段名各不相同(scale 看 `new_replicas`,rollback 看 `new_revision`…),与其要求每个 handler 记住这张映射表,不如宿主按 action 统一合成(真实代码):

```rust
// 据 action_id 从合并后的 attrs 合成 verifier 期望的 result 字段
fn synthesize_result_field(action_id: &str, merged_attrs: &Value) -> Option<Value> {
    let pick = |k: &str| merged_attrs.get(k).cloned();
    match action_id {
        "scale_deployment" => pick("desired_replicas").map(|v| json!({ "new_replicas": v })),
        "restart_pod"      => pick("restart_count").map(|v| json!({ "new_restart_count": v })),
        "refresh_secret"   => pick("secret_version").map(|v|   json!({ "new_version": v })),
        "rollback_deployment" => pick("current_revision").map(|v| json!({ "new_revision": v })),
        // ...
    }
}
```

修完给这两个纯函数补了单测 —— 不启动任何 WASM 就能测,因为它们本来就与绑定层解耦。复盘三条:

1. **mock 的「全绿」不等于接通了** —— mock 双方共享同一个心智模型,真实对接的两端不共享;
2. **两层各自写同一个对象时,合并语义是必须显式声明的契约**,不是实现细节;
3. 排查最好的产物不是那处修复,是把隐式约定变成代码里的显式映射。

坦白说,这三个 bug 在此前的实现里一直是潜伏的 —— real 模式那条路从未真正跑通过,直到这次带真集群的端到端验证才现形。这也是我一直坚持「验证要接真数据源」的原因:有些契约 bug,合成环境里永远绿。

## 小结

- 恢复动作的信任是**分层买来的**:dry-run 买「预演」,审批门买「授权」,verifier 买「确认」,自动回滚买「反悔保险」—— 少一层,按钮就悬;
- 预演和验证都做成**纯函数**(吃拓扑快照),才可能在测试里、在审批前反复跑;
- **mutable twin** 同时解决了「验证什么」和「回滚基于什么状态反转」两个问题,且天然隔离了模拟与真实;
- 回滚跳过二次审批、链只审一次 —— 审批的目的是控风险,不是走流程;
- 跨层写同一对象时,合并语义要写成显式契约,不然它会在最深的层咬你。

完整实现(`engine/crates/engine-recovery/`:action_defs / cascade / execution / verifiers / chains):**https://github.com/Thneoly/sre-inspection-graph**

> 系列上一篇:[Identity Resolution](./04-identity-resolution.md) ｜ 下一篇:[变更与告警的时间线](./06-change-tracking-timeline.md)
