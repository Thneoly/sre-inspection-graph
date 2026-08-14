# 用 WebAssembly 给不可信插件上镣铐:capability 沙箱模型实践

> 我的 SRE 图谱工具里,connector 是插件 —— 会跑「用户指定的代码」、要访问「生产集群」的东西。这篇讲我怎么用 WASM Component Model + deny-by-default capability,让插件既干得了活、又翻不了墙。

## 问题:插件是天然的攻击面

先看 connector 在我的架构里干什么:

- 拿到用户的配置(api_base、namespace、扫描目录……)
- 去访问外部系统:K8s API、Prometheus、Jaeger、本地文件系统
- 把结果吐回来变成图谱上的节点和边

也就是说,一个 connector **同时具备**「执行任意逻辑」和「触达敏感资源」两个属性。如果我把它做成进程内原生模块,一个恶意或有 bug 的 connector 可以读 `~/.kube/config`、扫 `/etc/passwd`、把集群凭据发到任意地址。

传统答案是用容器或子进程隔离。但 connector 的形态是「在一个桌面进程里被编排、每 30 秒跑一批的短任务」—— 为每个 connector 拉容器太重,子进程又让数据来回序列化变麻烦。我选了第三条路:

**WASM Component Model:插件编译成 `wasm32-wasip2` 组件,跑在宿主的 wasmtime 里;它想碰任何外部资源,只能通过宿主注入的 capability 函数 —— 宿主不给,它就碰不到。**

## deny-by-default:从 manifest 开始

每个 connector 在 manifest 里**显式申明**自己要什么能力,没申明的一律没有:

```toml
# modules/manifest.toml(节选)
[[modules]]
name = "k8s"
type = "connector"
wasm_path = "target/wasm32-wasip2/release/k8s.wasm"
capabilities = ["logging", "clock", "http-client"]   # ← 显式申明
config = { api_base = "http://127.0.0.1:8001", cluster = "vm-cluster", namespace = "otel-demo" }

[[modules]]
name = "code-repo"
type = "connector"
capabilities = ["logging", "clock", "fs-read"]        # ← 换成文件系统
fs_roots = ["/home/me/code/otel-demo"]                # ← 且限定根目录
```

宿主加载模块时把 capabilities 收进一个 `HashSet`。注意这是 **deny-by-default**:清单里没写的,host 端的实现直接拒绝。

## WIT:能力即接口

能力不是嘴上说说,是 WIT(WebAssembly Interface Types)里定义的真实接口。宿主和插件之间唯一的通道长这样(示意):

```wit
interface http-client {
    record http-response {
        status: u16,
        body: string,
    }
    variant http-error {
        unauthorized,
        not-found,
        timeout,
        other(string),
    }
    get: func(url: string) -> result<http-response, http-error>;
}

interface fs-read {
    record fs-entry {
        path: string,
        is-dir: bool,
    }
    variant fs-error {
        permission-denied,
        not-found,
        io(string),
    }
    read-file: func(path: string) -> result<string, fs-error>;
    read-dir: func(path: string) -> result<list<fs-entry>, fs-error>;
}
```

插件代码里调用的是类型安全的绑定,宿主端(我的 Rust 侧)实现这些接口。**插件没有 raw WASI 的文件/网络 preopen** —— 它能看到的文件系统,就是宿主愿意给它看的那两个函数。

## call-time 拒绝,而不是 link-time

一个实现细节值得展开:capability 校验我放在**每次调用时**查表,而不是加载时一次性链接/剪裁。

```rust
// 宿主侧伪码:http-client 的 call-time 门禁
impl HttpClientHost for State {
    fn get(&mut self, url: String) -> Result<HttpResponse, HostHttpError> {
        if !self.capabilities.contains("http-client") {
            return Err(HostHttpError::Unauthorized);   // ← 没申明,当场拒绝
        }
        self.http_get(&url)                            // 纯函数实现,可单测
    }
}
```

为什么选 call-time?两个原因:

1. **简单**:共享一个 Linker,不用为不同插件生成不同链接配置;
2. **可演进**:以后要加「URL allow-list」(比如 k8s connector 只许访问 api_base 前缀),只是往这个函数里加一层判断,模型不变。

代价是每次调用多一次 `HashSet` 查询 —— 对每 30 秒一批的轮询任务,完全无感。

这个设计有个关键分离:**capability 的逻辑(`http_get`、路径校验)写成纯函数,与 WIT 绑定层解耦**。所以我可以不启动任何 WASM 就单测「401 → Unauthorized、404 → NotFound、超时 → Timeout」这些语义,也能写「不申明 http-client 的插件整轮 0 fact + 拿到 unauthorized 错误」的端到端测试。

## fs-read:第一天就把路径逃逸堵死

文件系统能力比网络更危险,因为目录穿越是老牌漏洞。code-repo connector 要扫描本地克隆的代码仓,规则从一开始就定死:

1. **path-root allow-list**:manifest 里 `fs_roots` 列出允许访问的绝对路径,**为空 = 无任何访问**(有 capability 没 roots 也一样拒绝);
2. **canonicalize 校验**:请求路径先 `std::fs::canonicalize`(解析 `..` 和符号链接到真实路径),再检查是否落在某个 root 的 canonical 路径**之下** —— 注意是组件级前缀比较,不是字符串 `starts_with`,不然 `/data/repos2` 会匹配 `/data/repos`;
3. **只读**:接口只有 `read-file` / `read-dir`,没有写、没有删。

```
请求: /home/me/code/otel-demo/../../.ssh/id_rsa
  ↓ canonicalize
真实: /home/me/.ssh/id_rsa
  ↓ 落在 /home/me/code/otel-demo 之下?
  ↓ 否 → PermissionDenied
```

符号链接逃逸同理 —— canonicalize 之后藏不住。这几条路径都有单测钉死(`../../etc/passwd`、符号链接跳出 root、空 roots)。

对比一下「直接给 WASI preopen 目录」:那等于给了该目录下的**完整读写删**,而且 WASI 的 preopen 语义里防不住程序自己 `canonicalize` 之后再探测。capability 接口让我把「能读什么」收敛成宿主代码里肉眼可审计的十行。

## 状态码语义:拒绝也是一种协议

插件拿到拒绝时,宿主返回的是**结构化错误**而不是 panic。http-client 把 401/403 映射为 `Unauthorized`、404 为 `NotFound`、超时为 `Timeout`,其余透传给插件自决。fs-read 统一 `PermissionDenied`。

这带来一个很实用的副作用:**故障也是可观测的**。我的 UI 上有个 connectors 管理页,某 connector 本轮 sync 产出 0 fact 时,错误列表里能看到「permission denied: fs-read not granted」—— 用户能立刻明白是配置问题不是玄学。而插件侧,「拿到 Unauthorized → 记一条 error note、返回 0 fact、不崩溃」是每个 connector 的标准行为。

## 效果与边界

现在 6 个 connector 各自持有最小能力:k8s / prometheus / jaeger / k8s-events 只要 `http-client`,code-repo 只要 `fs-read`,flagd 走 `http-write`(POST 一个 ResolveAll)。加一种新能力 = WIT 加一个 interface + 宿主加一个 impl,插件侧按需申明。

诚实说边界:这套模型防的是**插件的能力逃逸**(它不该碰的东西),不防宿主自己把凭据喂给它 —— 比如 k8s connector 经 kubectl proxy 访问明文 API,TLS 和认证都在 proxy 那层,这是我刻意的架构决策(凭据不过 WASM 边界),但意味着信任链里有「proxy 是本地的」这个前提。安全是分层的,沙箱只是其中一层。

## 小结

- 插件化架构的第一天就要回答「插件凭什么被信任」,**deny-by-default 是最省心的默认答案**;
- WASM Component Model + WIT 让「能力」成为类型系统里的接口,而不是文档里的君子协定;
- call-time 校验比 link-time 简单,且不妨碍后续精细化(URL/path allow-list);
- 文件能力第一天就上 canonicalize + root 前缀校验 —— 目录穿越这种二十年老漏洞,没理由再中一次;
- 把能力实现写成与绑定层解耦的纯函数,安全逻辑才能像业务逻辑一样被单测覆盖。

完整实现(wasmtime host + 两个 capability + e2e 测试)在仓库里:**https://github.com/Thneoly/sre-inspection-graph**(`engine/crates/engine-wasm/`)

> 系列上一篇:[一个人从 Rust 内核做到 React 前端](./01-fullstack-sre-graph-tool.md) ｜ 下一篇:[canonical Fact 与三层数据契约](./03-canonical-fact-data-contract.md)
