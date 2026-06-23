# 12 — PRD-006 代码仓数据源接入 (Code Repository Source)

> **状态**:设计阶段(待评审)
> **依赖**:PRD-005(Fact 总线、Identity Resolver、Unknown Dep Queue)、PRD-002(ChangeEvent + 关联机制)
> **影响范围**:新增 1 个 connector + 1 个静态分析子模块 + 2 个 webhook 端点 + 4 个 ChangeEvent 类型 + 1 个新前端页

---

## 1. 背景

PRD-002 Phase 2 已经从下游接入了部署侧变更(ArgoCD webhook → `deployment_rolled`、Harbor → `image_pushed`),但**变更链路的上游**仍是缺失的:

- ❌ PR/MR 还没合入时的预审 — 无法预演影响面
- ❌ 主干 push 但还没触发 ArgoCD 同步的窗口 — 关联告警找不到诱因
- ❌ Repo 本身的元数据(谁拥有、什么 topic、依赖什么库) — 节点 owner_team 字段全靠手工
- ❌ 代码内容的语义分析 — 业务规则 / SLO / 特性开关定义全部沉睡在文件里

**代码仓是这些信息的天然存储位置**。本 PRD 把代码仓作为 PRD-005 Fact 总线的一类数据源接入,让平台从"运维感知"升级到"运维感知 + 人类意图感知"。

---

## 2. 代码仓能贡献的 5 类数据

| 类别 | 代码仓里的形态 | 变成什么 |
|---|---|---|
| **1. 资产元数据** | repo 列表 / 描述 / Owner / Topic / License | `CodeRepo` 节点 + 元数据 |
| **2. 构建产物映射** | `Dockerfile` / `charts/` / `k8s/*.yaml` / `Skaffold` | `Repo -BUILDS-> Image` 等边 |
| **3. 依赖清单**(静态) | `package.json` / `go.mod` / `pom.xml` / `requirements.txt` + lock | `Repo -DEPENDS_ON-> Library` 边 + 漏洞继承 |
| **4. 事件流**(运行时) | PR/MR open / review / merge / tag release | `ChangeEvent`(扩 `pr_merged` 等类型) |
| **5. 业务规则**(语义) | threshold 常量 / SLO 定义 / 业务校验函数 / 配置 YAML / FF 定义 | `InspectionRule` / `AlertRule` / 节点 properties 富化 |

第 5 类是其他数据源**完全给不了**的能力 —— 单独立项介绍。

---

## 3. 新增节点 / 边类型

### 3.1 节点

| 节点 type | ID 模式 | 关键字段 |
|---|---|---|
| `CodeRepo` | `repo:<host>:<group>:<name>` | `git_url, default_branch, language, topics, owner_team, last_commit_at` |
| `Library` | `pkg:<ecosystem>:<name>@<version>` | `ecosystem(npm/maven/go/pypi), name, version, cve_count, license` |
| `Pipeline` | `pipeline:<host>:<group>:<name>:<branch>` | `repo_id, branch, schedule, last_run_status` |

### 3.2 边

| 边类型 | 起点 | 终点 | 含义 |
|---|---|---|---|
| `BUILDS` | `CodeRepo` | `ContainerImage` | 此 repo 产生此镜像 |
| `DEFINES` | `CodeRepo` | `Deployment` / `Service` / `ConfigMap` | 此 repo 的 YAML 声明此 K8s 资源 |
| `DEPENDS_ON` | `CodeRepo` | `Library` | 静态依赖 |
| `EXTRACTED` | `CodeRepo` | `InspectionRule` | 从代码抽出的规则 |
| `APPLIES_RULE` | `Component` | `InspectionRule` | 组件适用此规则 |
| `OWNS` | `Team` | `CodeRepo` | 团队拥有 |

---

## 4. ChangeEvent 扩展

`change_type` 枚举从 4 → 8:

```python
# backend/app/datasource/models.py
ChangeEvent.change_type ∈ {
    # 现有 4 种(PRD-002)
    "configmap_updated", "secret_rotated", "deployment_rolled", "image_pushed",
    # PRD-006 新增 4 种
    "pr_opened",       # PR/MR 创建,带预期影响面预演
    "pr_merged",       # 合入,真实 commit_sha + diff
    "release_tagged",  # 打 tag,带语义版本号
    "direct_push",     # 绕过 PR 直推主干(severity 自动 medium)
}
```

**字段 100% 复用 PRD-002**:`commit_sha / pipeline_url / git_repo / yaml_diff` 等已就绪;新增字段:

```python
# PRD-006 新增字段
pr_id: str = ""              # GitLab MR IID / GitHub PR number
pr_title: str = ""
pr_author: str = ""
pr_reviewers: list[str] = field(default_factory=list)
files_changed: list[str] = field(default_factory=list)
loc_added: int = 0
loc_removed: int = 0
```

**关联机制零改动**:PRD-002 的 `correlate_alerts` + `correlate_and_persist` 自动跟近窗口告警建 `CORRELATED_WITH` 边 — PR 合入后近 1h 内的告警自动归因。

---

## 5. code_repo_connector 实现

### 5.1 拉数据(GitLab 为主、GitHub 为辅、Gitea 兼容)

```python
# backend/app/datasource/connectors/code_repo/code_repo_connector.py

class CodeRepoConnector(BaseConnector):
    name = "code_repo"
    sync_interval_seconds = 1800   # 30 min,webhook 主路径 + 兜底轮询

    async def sync_once(self) -> SyncResult:
        # 1) GET /api/v4/projects?membership=true&per_page=100  (GitLab)
        repos = await self._list_repos()
        for repo in repos:
            await self._publish_repo_fact(repo)             # CodeRepo 节点
            await self._publish_deps_facts(repo)            # 依赖清单
            await self._publish_build_facts(repo)           # Dockerfile/charts 解析

        # 2) 兜底拉漏的 MR(webhook 丢的)
        # GET /api/v4/projects/:id/merge_requests?updated_after=...
        missed = await self._list_missed_mrs()
        for mr in missed:
            await record_change(...)  # 复用 PRD-002

        return SyncResult(...)
```

### 5.2 构建产物映射解析

按文件类型识别 + 解析(纯函数,可单测):

```python
# backend/app/datasource/connectors/code_repo/build_parser.py

def parse_dockerfile(content: str) -> list[ImageRef]:
    """从 Dockerfile FROM/COPY 推导基础镜像"""

def parse_k8s_manifest(yaml_text: str) -> list[K8sResourceRef]:
    """从 k8s/*.yaml 推导声明的 Deployment/Service/CM 名"""

def parse_helm_chart(chart_yaml: str, values_yaml: str) -> list[HelmRef]:
    """解析 charts/<name>/Chart.yaml + values.yaml"""

def parse_argocd_app(app_yaml: str) -> ArgoAppRef:
    """ArgoCD Application CR → repoURL/path/destination"""
```

每个解析结果发 Fact:
```python
TopologyFact(
    source="code_repo_connector",
    fact_type="edge",
    correlation_keys=["image:registry.local/order/order-svc"],
    payload={
        "from": "repo:gitlab:order:order-service",
        "to":   "image:registry.local/order/order-svc",
        "type": "BUILDS",
    },
)
# → Identity Resolver 用 image: key 合并到 Harbor connector 建的 Image 节点
```

### 5.3 依赖清单解析

```python
# backend/app/datasource/connectors/code_repo/deps_parser.py

PARSERS = {
    "package.json":      parse_npm,        # + package-lock.json
    "go.mod":            parse_go,          # + go.sum
    "pom.xml":           parse_maven,
    "requirements.txt":  parse_pypi,
    "Cargo.toml":        parse_rust,
}

def parse_npm(content: str, lock: str) -> list[LibraryRef]:
    """返回 [(name, version, scope)]"""
```

Library 节点用 [purl](https://github.com/package-url/purl-spec) 风格 ID:`pkg:npm/lodash@4.17.21`;CVE 关联走现有 ContainerImage 的 `image_risk` 视图复用思路。

### 5.4 Webhook(主路径)

```python
# backend/app/routers/webhook.py 扩展

@router.post("/gitlab")    # X-Gitlab-Event: Merge Request Hook / Push Hook
async def gitlab_webhook(body: dict, x_gitlab_token: str | None = Header(None)):
    _check_token(x_gitlab_token)
    event_type = body.get("object_kind")
    if event_type == "merge_request":
        await _handle_mr(body)
    elif event_type == "push":
        await _handle_push(body)
    elif event_type == "tag_push":
        await _handle_tag(body)
    return {"ok": True}

@router.post("/github")    # X-GitHub-Event: pull_request / push
async def github_webhook(...): ...
```

`_handle_mr` 内部直接调 `record_change(change_type="pr_opened" 或 "pr_merged", ...)`,**自动走 PRD-002 那套**:propagation BFS、severity 估计、AlertEvent 关联。

---

## 6. 业务规则抽取(第 5 类数据 — 独立子模块)

这是 PRD-006 的差异化能力。分三批上,**S1 用 regex 拿 80% ROI**,S2-S3 是可选升级。

### 6.1 S1:Regex / grep 派(本期)

```yaml
# backend/app/datasource/connectors/code_repo/rules/regex_rules.yaml

- name: threshold_constant
  description: Spring @Value / Go env 常量
  pattern: '@Value\("\$\{([\w.]+):(\d+)\}"\)'
  language: [java, kotlin]
  capture: [key, value]
  emit:
    fact_type: rule
    rule_type: threshold

- name: slo_annotation
  description: 自定义 @SLO 注解
  pattern: '@SLO\(p99_ms\s*=\s*(\d+)\)'
  capture: [p99_ms]
  emit:
    fact_type: rule
    rule_type: slo

- name: feature_flag_def
  description: flagd / OpenFeature 定义点
  pattern: 'FeatureFlag\.enable\("(\w+)"\)|flagd.*flag\s+(\w+)\s*='
  capture: [flag_name]
  emit:
    fact_type: rule
    rule_type: flag_def

- name: outbound_http_call
  description: 硬编码的外部 URL
  pattern: '(https?://[\w.-]+(?::\d+)?(?:/[\w./?=&-]*)?)'
  filter_out: ['localhost', '127.0.0.1', '.local', '.svc']
  emit:
    fact_type: rule
    rule_type: outbound_call
```

### 6.2 抽出来的规则放到图谱

```
CodeRepo ──EXTRACTED─→ InspectionRule
  │                       │
  │                       ├─ rule_type: threshold | slo | flag_def | rbac | outbound_call
  │                       ├─ source_file: src/main/java/.../OrderService.java
  │                       ├─ source_line: 233
  │                       ├─ source_commit: abc1234
  │                       ├─ raw_snippet: "..."
  │                       ├─ confidence: 0.6        # regex 派偏低
  │                       └─ extracted_at: ...
  │
  └─DEFINES─→ Component(order-service)
                │
                └──APPLIES_RULE──→ 同一个 InspectionRule
                     (双向边,Component 视角能看到所有适用规则)
```

**InspectionRule 节点本就在 L4 模型里**,从代码抽规则等于给 L4 找了**稳定内容供应商**(以前只能人工写)。

### 6.3 S2/S3(本期外)

- **S2 — AST + tree-sitter**(4 周):跨函数引用、注解参数、SLO 装饰器
- **S3 — LLM 辅助**(2 周接入 + 持续运维):对复杂业务规则用 LLM,输出 JSON,人工 review

---

## 7. 与 Unknown Dependency Queue 联动(PRD-005 × PRD-006)

**这是 PRD-006 最有意思的能力**:

```
Trace 看到:                          代码仓 grep:
"endpoint:pay-svc.io:443 调用 8k 次"  → 在 order-service repo 里找到
                                       file: PaymentClient.java:34
                                       code: feign.target("https://pay-svc.io")

              ↓ 自动联动
   Unknown Dep Queue 富化:
   "endpoint:pay-svc.io:443
    被调用方:外部 SaaS(自动推测)
    调用来源:repo:order-service:main/.../PaymentClient.java:34
    最后改动:commit abc1234 (alice@ 2026-06-15)
    建议:确认是否合规依赖,若是 → 建 ExternalSaaS 节点;若不是 → 联系 alice"
```

实现:`unknown_dep.enrich()` 加 `_grep_in_repos(endpoint)` 步骤,查 outbound_call 规则索引找匹配项,把 file/line/commit 富化进 unknown_dep 记录。

---

## 8. 与现有 PRD 的关系

| 现有 PRD | 代码仓接入后怎么变 |
|---|---|
| **PRD-001 恢复动作** | 多一个 `revert_pr` 动作:从 ChangeEvent(pr_merged) → 调 GitLab API 自动产 revert PR → 触发 ArgoCD 同步。和 `rollback_deployment` 互补(后者回滚镜像版本,前者回滚代码) |
| **PRD-002 ChangeEvent** | 直接受益最大,事件源从 K8s/Argo/Harbor 扩到代码仓上游,**完整变更链路** |
| **PRD-003 自检报告** | 新模板 `code_quality_report`:本周 N 次 direct_push / M 个 pr_merged 关联了 P1 / Q 个 repo 用了 log4j 2.14。**报告内容自动从图里查**,不写新代码 |
| **PRD-004 Connector** | 加一个 connector(`code_repo_connector`),走 BaseConnector 标准接口 |
| **PRD-005 UTS** | 直接消费 Fact 总线 + Identity Resolver,**总线层零改动** |
| **L4 InspectionRule** | 从"人工写规则"升级到"代码自动产规则",空白的 L4 层第一次有持续数据流 |

---

## 9. 实施路线 — 2 个 Sprint

### Sprint 1 — 基础接入(2 周)

- `code_repo_connector.py` — GitLab Open API 优先,GitHub 次
- 拉 repo 列表 → 发 `CodeRepo` 节点 Fact
- 解析 `Dockerfile` / `k8s/*.yaml` / `charts/` → 发 `BUILDS` / `DEFINES` 边 Fact
- 依赖清单解析(`package.json` / `go.mod` / `pom.xml`)→ `Library` 节点 + `DEPENDS_ON` 边
- Webhook 端点 `/webhooks/gitlab` / `/webhooks/github` → 复用 `record_change()`
- 扩 `ChangeEvent.change_type` 加 4 个值 + 新增 PR 元数据字段
- 前端:`NodeDetailPanel` 给 Component 节点加「代码仓」信息卡(repo URL / 最近 5 个 PR)

### Sprint 2 — 语义抽取 + UnknownDep 联动(2 周)

- Regex 规则引擎(YAML 配置文件 + 简单匹配器)
- 抽 5 种规则:`threshold` / `slo` / `flag_def` / `rbac` / `outbound_call`
- 抽出来的 Fact → `InspectionRule` 节点 + `EXTRACTED` / `APPLIES_RULE` 边
- Unknown Dep Queue × 代码仓联动(`_grep_in_repos` 富化)
- 前端:`/rules` 新页面,列代码抽出来的所有规则,可点击跳代码行
- 前端:`/topology/unknown-deps` 卡片显示 "来自 repo:xxx:line:yyy" 字样

### Sprint 3+(本期外)

- AST 解析(tree-sitter)替代 regex
- LLM 辅助抽业务规则(走自建模型 / 私有部署,慎用)
- `revert_pr` 恢复动作(PRD-001 扩展)
- `code_quality_report` 报告模板(PRD-003 扩展)

---

## 10. 验收标准

### Sprint 1
- [ ] 现网 GitLab 上的 10+ 个 repo,uvicorn 启动 30min 后全部以 `CodeRepo` 节点入图
- [ ] Repo 节点带 `language / topics / owner_team / last_commit_at` 属性
- [ ] 解析 `Dockerfile` 后,Repo → Image 边创建,且 Image 节点跟 Harbor connector 产的同一 ID
- [ ] 测试 repo 创建 PR → webhook 5s 内收到 → `ChangeEvent(pr_opened)` 入 DSS
- [ ] PR 合入 → `ChangeEvent(pr_merged)` 入 DSS,且 commit_sha / files_changed / pr_author 完整
- [ ] PRD-002 `correlate_alerts` 自动跑,1h 窗口告警关联到该 ChangeEvent

### Sprint 2
- [ ] 测试 repo `OrderService.java:233` 写 `@Value("${order.timeout:30}")`,扫描后图里出现 `InspectionRule(threshold, key=order.timeout, value=30)` 节点
- [ ] `Component(order)` 节点 detail 页可看到 "适用规则" 列表
- [ ] 测试调用一个外部 URL(代码 grep 能找到)→ Unknown Dep Queue 记录里富化字段 `callers_from_code=[{file, line, commit}]`

---

## 11. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 大 repo 全文扫描慢 | 增量扫:首次只扫 "关键路径"(Dockerfile / k8s yaml / config / src 的关键模式);后续只扫 PR 改动的文件 |
| GitLab/GitHub API rate limit | webhook 主路径 + 30min 兜底轮询;不做 5s 拉一次;分页 + 增量 since 参数 |
| 抽出来的规则误报 | 所有 LLM/AST 产的规则带 `confidence` 字段,前端展示;**InspectionFinding 不自动告警,只列出**让 SRE 人工确认 |
| 代码 PII / 商密泄漏 | regex / AST 都本地跑,不发外网;LLM 走自建模型 / 私有部署 |
| Monorepo / 多 fork | repo_id 用 `<host>:<group>:<name>`,monorepo 内多 service 用 `repo_path` 子路径区分 |
| 历史 commit 回溯 | **不做** — 只扫 default branch HEAD;历史是无底洞 |
| Webhook 签名校验缺失 | 复用 PRD-002 webhook 的 `_check_token` 模式;`WEBHOOK_TOKEN` env 兜底,生产必开 |

---

## 12. 不做(本期外)

| 能力 | 延后到 |
|---|---|
| AST / tree-sitter 解析 | S3 |
| LLM 抽业务规则 | S3+ |
| 提交历史回溯 | 永远不做 |
| 代码所有权图(blame 派生) | Phase 4 |
| PR 影响面**预演**(合入前 dry-run) | Phase 4(需 propagation 算法支持假设场景) |
| 自动 revert PR | PRD-001 扩展 |
| 跨 repo 引用追踪(monorepo + 外部依赖) | Phase 4 |

---

## 13. File Map(实施后)

```
backend/app/
├── datasource/
│   └── connectors/
│       └── code_repo/                           # PRD-006 新模块
│           ├── code_repo_connector.py           # BaseConnector 子类
│           ├── gitlab_client.py                 # GitLab Open API 封装
│           ├── github_client.py                 # GitHub 封装(可选)
│           ├── build_parser.py                  # Dockerfile / charts / k8s yaml 解析
│           ├── deps_parser.py                   # package.json / go.mod / pom.xml
│           ├── rule_extractor.py                # regex 引擎
│           └── rules/
│               └── regex_rules.yaml             # 规则配置
├── routers/
│   ├── webhook.py                               # 扩 /gitlab + /github 端点
│   └── rules.py                                 # 新:/api/v1/rules(列代码抽规则)
└── changes/
    └── event_service.py                         # record_change 扩 PR 元数据

frontend/src/
├── components/
│   ├── Graph/
│   │   └── NodeDetailPanel.tsx                  # 加"代码仓"信息卡
│   └── Views/
│       └── RulesView.tsx                        # 新:/rules 页面
└── api/client.ts                                # CodeRepo / Library / Rule 类型 + API

backend/tests/
├── test_code_repo_connector.py                  # ~20 tests
├── test_build_parser.py                         # Dockerfile/k8s/helm 解析单测
├── test_deps_parser.py                          # 各语言依赖解析
└── test_rule_extractor.py                       # regex 规则单测

frontend/src/__tests__/
└── RulesView.test.tsx                           # ~5 tests
```

---

## 14. 一句话总结

PRD-006 把代码仓作为 PRD-005 Fact 总线的一类数据源接入,贡献 5 类数据(资产元数据 / 构建映射 / 依赖清单 / PR 事件 / **业务规则**),**总线层零改动**。**S1 用 regex 抽规则就能拿 80% ROI**,把"运维感知"补上"为什么"这一维;与 PRD-005 Unknown Dependency Queue 联动,让 trace 看到的外部调用能自动回溯到代码行 — 这是其他数据源给不了的能力。
