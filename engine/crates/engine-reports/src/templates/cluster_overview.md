# 集群健康总览 - {{ scope.cluster_id | default(value="全公司") }}

- **模板**:cluster_overview
- **生成时间**:{{ generated_at }}
- **报告 ID**:{{ report_id }}
- **范围**:{{ scope.cluster_id | default(value="全部集群") }}
- **时间区间**:{{ scope.time_range_start | default(value="-") }} ~ {{ scope.time_range_end | default(value="-") }}

---

{% if modules.cluster_health %}
## 1. 健康分布

应用总数:**{{ cluster_health.total_apps }}**

| 评级 | 数量 |
|---|---|
| 健康 | {{ cluster_health.rating_counts.healthy }} |
| 健康警告 | {{ cluster_health.rating_counts.health_warning }} |
| 风险中 | {{ cluster_health.rating_counts.risk_medium }} |
| 风险高 | {{ cluster_health.rating_counts.risk_high }} |

### 应用评分(由低到高)

{% if cluster_health.apps %}
| 应用 | 评分 | 评级 |
|---|---|---|
{% for a in cluster_health.apps %}
| **{{ a.name }}** (`{{ a.application_id }}`) | {{ a.score }} | {{ a.rating }} |
{% endfor %}
{% else %}
无应用数据。
{% endif %}
{% endif %}

{% if modules.cluster_risk_top_n %}
## 2. 风险 Top-N

Top **{{ cluster_risk_top_n.top_n }}** 风险应用 + 全局指标:

| 应用 | 评分 | 评级 |
|---|---|---|
{% for a in cluster_risk_top_n.top_apps %}
| **{{ a.name }}** | {{ a.score }} | {{ a.rating }} |
{% endfor %}

- 活跃故障:**{{ cluster_risk_top_n.active_faults_total }}**(Rust 版无 fault injection)
- 高危变更:**{{ cluster_risk_top_n.high_severity_changes_total }}**
{% endif %}

{% if modules.cluster_changes %}
## 3. 跨应用变更汇总

范围内共 **{{ cluster_changes.total }}** 次变更。

**变更类型分布**:
{% for k, v in cluster_changes.by_type %}- {{ k }}:{{ v }}
{% endfor %}

**Top-5 受变更最多的资源**:
{% if cluster_changes.top_targets %}
| 资源 | 变更数 |
|---|---|
{% for t in cluster_changes.top_targets %}
| `{{ t.resource_id }}` | {{ t.changes }} |
{% endfor %}
{% else %}
无变更数据。
{% endif %}
{% endif %}

{% if modules.cluster_recoveries %}
## 4. 跨应用恢复执行

范围内共 **{{ cluster_recoveries.total }}** 次恢复执行。

| 状态 | 数量 |
|---|---|
{% for k, v in cluster_recoveries.status_counts %}| {{ k }} | {{ v }} |
{% endfor %}

**成功率**:{{ cluster_recoveries.success_rate }}
{% endif %}

---

*由 SRE 巡检图谱平台自动生成 - PRD-003 Phase 4.2a(Markdown)*
