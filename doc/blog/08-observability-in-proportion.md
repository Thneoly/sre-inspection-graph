# 给观测工具自己做观测:按比例的 observability

> 做了个 SRE 观测平台,然后被自己问倒:「那你自己可观测吗?」日志只打在用户看不见的 stdout、sync 慢了不知道慢在哪、connector 是一直失败还是今天才开始失败也答不上。这篇讲怎么补 —— 以及更重要的,怎么决定**补到哪为止**。

## 先问消费者是谁

「给自己的工具上 OTel」的第一反应可能是:接 OpenTelemetry SDK、起 Collector、导 metrics/traces/logs。但先问一句:**这些信号的消费者是谁?**

- 单用户桌面 app,用户就是操作者 —— 没有值班团队盯着 dashboard
- 没有第二个服务消费这些信号 —— exporter 导出去也没人收
- 日志、指标、链路的「集中式」价值来自多服务聚合 —— 这里只有一个进程

结论:**全套 OTel 在这个形态里是过度工程**。观测要按系统的形态配比 —— 这和这个项目一贯的原则同源([Identity Resolution 那篇](./04-identity-resolution.md)的「不为演示造合成问题」):不为不存在的消费者建机制。

但「不上全套」不等于「不做」。盘点下来有三个真实缺口,各补最小的一件。

## 缺口一:日志只活在 stdout 里

桌面 app 的 tracing 日志默认打到 stdout —— 而桌面用户根本不打开终端。可调试性的及格线是「用户能把日志文件发给你」,一条都达不到。

补法:`tracing-appender` 按天滚动的文件 appender,和 stderr **双写**(开发时不丢终端输出)。挂在现有 tracing 上,零新架构:

```rust
let appender = tracing_appender::rolling::daily(&log_dir, "sre-graph.log");
let _ = tracing_subscriber::fmt()
    .with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    )
    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
    .with_writer(
        tracing_subscriber::fmt::writer::MakeWriterExt::and(std::io::stderr, appender),
    )
    .try_init();
```

一个坑:appender **只轮转、不清理** —— 文件会无限累积。保留期自己管,启动时扫一遍删过期文件。为了能测「13 天留、15 天删」的边界,把 `now` 作为参数注入(而不是函数里取真实时间):

```rust
fn prune_old_logs_with_now(dir: &std::path::Path, keep_days: u64, now: std::time::SystemTime) {
    let cutoff = now - std::time::Duration::from_secs(keep_days * 86_400);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_file() && md.modified().map(|m| m < cutoff).unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}
```

「时间注入」是个性价比极高的测试手法:凡是依赖当前时间的逻辑(保留期、TTL、窗口),把 now 参数化,边界测试就不用伪造文件时间戳这种脏活。

## 缺口二:sync 慢了,不知道慢在哪

一次 sync 是条管线:拉数据 → 落库 → resolve(含多源合并)→ diff → 变更检测 → 物化。哪段慢?现在只有总耗时和 per-connector 计时,管线内部是黑盒。

既然全项目已在用 `tracing`,补的是 span,不是新系统。engine 侧两个函数加属性:

```rust
#[tracing::instrument(skip(self, config_json), fields(connector = %self.name))]
pub async fn run_sync(&self, config_json: &str) -> SyncOutcome { ... }
```

desktop 侧整条 `run_sync` 一个 span,内部四个阶段各一个子 span。这里有个 **Rust 异步的陷阱值得记**:直觉写法是在阶段前后 `info_span!(...).entered()` 拿 guard —— 但 guard 跨 `.await` 不是 `Send`,而 Tauri 命令要求 future 是 Send,直接编译失败。正确姿势是用 `Instrument` **包 future**:

```rust
tracing::Instrument::instrument(
    state.storage.apply_change_set(&change_set),
    tracing::info_span!("stage: apply_changes"),
)
 .await
```

`FmtSpan::CLOSE` 开着,span 关闭时输出一行 `time.busy / time.idle` —— 慢在哪一段,翻日志文件就有。span 树 + 文件落盘,两件补在一起才完整:树负责「哪段」,文件负责「事后可查」。

## 缺口三:只有快照,没有趋势

connector 管理页能答「最近一轮 sync 产出多少、有没有错」,但答不了 **「prometheus 是一直失败,还是今天才开始失败?」** —— 状态是单值快照,没有历史。

补法克制到近乎简陋:注册表里给每个 connector 挂一个**环形缓冲**,每轮 sync 追加一个采样,超长截头:

```rust
s.history.push(SyncSample {
    synced_at: now_iso.clone(),
    fact_count: pcs.fact_count,
    duration_ms: pcs.duration_ms,
    error_count: pcs.errors.len(),
});
if s.history.len() > SYNC_HISTORY_LEN {
    let excess = s.history.len() - SYNC_HISTORY_LEN;
    s.history.drain(..excess);
}
```

12 轮 × 默认 30s 间隔 ≈ 6 分钟窗口。前端在展开行画趋势条 —— **纯 CSS div**,高度正比耗时、绿色正常红色有错、悬停看明细,零图表库依赖。内存里 12 条 JSON,不落盘、不建时间序列库 —— 它只需要回答「刚才那几分钟长什么样」,仅此而已。

这是一次显式的**快照→准时间序列**升级,且升级幅度对齐需求幅度:真要「过去 30 天的 connector 健康」,正确答案是把采样写进已有的 SQLite,而不是上 Prometheus。

## 刻意不做的三件

和做了什么同样重要的是清单:

1. **OTLP / metrics exporter** —— 没有第二个消费者。哪天这个工具服务化了(多人共用、集中部署),再接不迟,而且到那时接入点也是清晰的(把 SyncSample 导出到 exporter,而不是重构)。
2. **wasmtime fuel 给插件做资源计量** —— 这是「第三方插件」场景的安全闸(能力闸、路径闸之后的第三道:时间闸)。现在 6 个 connector 都是自己写的,没有失控威胁。
3. **自观测 dogfood**(把 sync 健康产成 fact 喂进自己的图)—— 故事漂亮,但按「何时不做」的原则问一句:真实消费者是谁?想清楚再加。

## 小结

- 观测的配比跟随系统形态:单用户桌面工具的服务化观测栈,是拿大炮轰蚊子;
- 三个真实缺口各补最小件:**日志落盘**(可调试性及格线)、**span 树**(慢在哪可查)、**环形采样**(快照变趋势);
- 两个顺手的手法:时间注入让保留期这类逻辑可测;`Instrument::instrument` 包 future 避开跨 await 持 span guard 的 Send 陷阱;
- 「不做什么」写下来和「做了什么」一样值钱 —— OTLP、fuel、dogfood 各自有清晰的触发条件,条件到了再做,不提前。

> 面试一句话版:「观测工具自己也需要观测,但按比例 —— 我补了日志落盘、sync 管线 span 树、connector 状态历史趋势三件,刻意不上 OTLP,因为没有第二个信号消费者;过度工程和观测缺失一样是债。」

实现(`desktop/src-tauri/src/lib.rs` 日志 + `commands/connectors.rs` 历史 + `engine-wasm` instrument):**https://github.com/Thneoly/sre-inspection-graph**

> 系列上一篇:[一个 subgraph 原语,六个巡检视图](./07-subgraph-views.md) ｜ 回到开篇:[一个人从 Rust 内核做到 React 前端](./01-fullstack-sre-graph-tool.md)
