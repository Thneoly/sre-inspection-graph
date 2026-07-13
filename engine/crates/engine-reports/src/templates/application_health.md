# 应用健康报告 - {{ scope.application_id }}

- **模板**:application_health
- **生成时间**:{{ generated_at }}
- **报告 ID**:{{ report_id }}
- **时间范围**:{{ scope.time_range_start | default(value="-") }} ~ {{ scope.time_range_end | default(value="-") }}

---

{% if modules.health_score %}
## 1. 健康度评分

**评分**:`{{ health_score.score }}` / 100  -  评级:**{{ health_score.rating }}**

| 指标 | 数量 |
|---|---|
| Critical 项 | {{ health_score.breakdown.critical }} |
| Warning 项 | {{ health_score.breakdown.warning }} |
| 故障 Pod | {{ health_score.breakdown.fault_pod }} |
| 子树节点总数 | {{ health_score.breakdown.total_nodes }} |

> 评分公式:100 - critical×10 - warning×3 - fault_pod×2(下限 0)。Rust 版无 fault injection,fault_pod=0。
{% endif %}

{% if modules.seven_views %}
## 2. 视图结论汇总

### 应用拓扑
应用包含 **{{ seven_views.topology.components }}** 个组件、**{{ seven_views.topology.deployments }}** 个 Deployment、**{{ seven_views.topology.pods }}** 个 Pod、**{{ seven_views.topology.services }}** 个 Service,子树共 **{{ seven_views.topology.total_nodes }}** 个节点,其中 **{{ seven_views.health.not_ready_pods }}** 个 Pod 未就绪。

### 健康分布
| 正常 | 告警 | 严重 |
|---|---|---|
| {{ seven_views.health.normal }} | {{ seven_views.health.warning }} | {{ seven_views.health.critical }} |

### 活跃故障
当前无活跃故障(Rust 版无 fault injection)。

### 变更统计(范围内)
范围内共 **{{ seven_views.changes.total }}** 次变更:{% for k, v in seven_views.changes.by_type %}{{ k }}: {{ v }}{% if not loop.last %}, {% endif %}{% endfor %}。

### 恢复执行
范围内共 **{{ seven_views.recoveries.total }}** 次恢复执行(成功 {{ seven_views.recoveries.succeeded }} / 失败 {{ seven_views.recoveries.failed }} / 已回滚 {{ seven_views.recoveries.rolled_back }})。
{% endif %}

{% if modules.risk_list %}
## 3. 风险清单

### 🔴 Critical({{ risk_list.counts.critical }} 项)
{% if risk_list.critical %}
{% for r in risk_list.critical %}
- **{{ r.name }}** (`{{ r.resource_id }}`,{{ r.resource_type }})- {{ r.reason }}
{% endfor %}
{% else %}
无 critical 风险。
{% endif %}

### 🟡 Warning({{ risk_list.counts.warning }} 项)
{% if risk_list.warning %}
{% for r in risk_list.warning %}
- **{{ r.name }}** (`{{ r.resource_id }}`,{{ r.resource_type }})- {{ r.reason }}
{% endfor %}
{% else %}
无 warning 风险。
{% endif %}

### 📋 高危变更({{ risk_list.counts.change }} 项)
{% if risk_list.change %}
{% for r in risk_list.change %}
- **{{ r.name }}** (`{{ r.resource_id }}`)- {{ r.reason }} @ {{ r.changed_at }}
{% endfor %}
{% else %}
无高危变更。
{% endif %}
{% endif %}

{% if modules.recommended_actions %}
## 4. 推荐恢复动作

{% if recommended_actions.actions %}
{% for a in recommended_actions.actions %}
{{ loop.index }}. **{{ a.action_id }}** -> `{{ a.target_resource_id }}`({{ a.source }})- {{ a.rationale }}
{% endfor %}
{% else %}
当前无推荐动作。
{% endif %}

> 动作执行请到「恢复」页面发起(关联 PRD-001)。
{% endif %}

{% if modules.historical_trends %}
## 5. 历史趋势(近 {{ historical_trends.days }} 天)

| 日期 | 变更数 | 恢复执行数 |
|---|---|---|
{% for row in historical_trends.rows %}
| {{ row.date }} | {{ row.changes }} | {{ row.recoveries }} |
{% else %}
| - | 0 | 0 |
{% endfor %}

合计:变更 **{{ historical_trends.total_changes }}** 次,恢复 **{{ historical_trends.total_recoveries }}** 次。
{% endif %}

---

*由 SRE 巡检图谱平台自动生成 - PRD-003 Phase 4.1(Markdown)*
