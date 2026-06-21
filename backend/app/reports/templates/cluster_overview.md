{# cluster_overview.md — PRD-003 Sprint 2 集群/全公司总览模板 #}
# 集群健康总览 — {{ scope.cluster_id or "全公司" }}

- **模板**:cluster_overview
- **生成时间**:{{ generated_at }}
- **报告 ID**:{{ report_id }}
- **范围**:{{ scope.cluster_id or "全部集群" }}
- **时间区间**:{{ scope.time_range_start or "—" }} ~ {{ scope.time_range_end or "—" }}

---

{% if modules.cluster_health %}
## 1. 健康分布

应用总数:**{{ cluster_health.total_apps }}**

| 评级 | 数量 |
|---|---|
| 健康 | {{ cluster_health.rating_counts["健康"] }} |
| 健康警告 | {{ cluster_health.rating_counts["健康警告"] }} |
| 风险中 | {{ cluster_health.rating_counts["风险中"] }} |
| 风险高 | {{ cluster_health.rating_counts["风险高"] }} |

### 应用评分(由低到高)

{% if cluster_health.apps %}
| 应用 | 评分 | 评级 |
|---|---|---|
{% for a in cluster_health.apps -%}
| **{{ a.name }}** (`{{ a.application_id }}`) | {{ a.score }} | {{ a.rating }} |
{% endfor %}
{% else %}
范围内未发现 Application 节点。
{% endif %}
{% endif %}

{% if modules.cluster_risk_top_n %}
## 2. 高风险 Top-{{ cluster_risk_top_n.top_n }} 应用

全局活跃故障:**{{ cluster_risk_top_n.active_faults_total }}** ;高危变更:**{{ cluster_risk_top_n.high_severity_changes_total }}**

{% if cluster_risk_top_n.top_apps %}
| # | 应用 | 评分 | 评级 |
|---|---|---|---|
{% for a in cluster_risk_top_n.top_apps -%}
| {{ loop.index }} | **{{ a.name }}** (`{{ a.application_id }}`) | {{ a.score }} | {{ a.rating }} |
{% endfor %}
{% else %}
范围内无应用。
{% endif %}
{% endif %}

{% if modules.cluster_changes %}
## 3. 变更汇总

总数:**{{ cluster_changes.total }}**

{% if cluster_changes.by_type %}
**按类型**:
{% for kv in cluster_changes.by_type | dictsort -%}
- {{ kv.0 }}:{{ kv.1 }}
{% endfor %}
{% endif %}

{% if cluster_changes.top_targets %}
**Top-5 受变更资源**:
| 资源 | 变更次数 |
|---|---|
{% for t in cluster_changes.top_targets -%}
| `{{ t.resource_id }}` | {{ t.changes }} |
{% endfor %}
{% endif %}
{% endif %}

{% if modules.cluster_recoveries %}
## 4. 恢复执行汇总

总数:**{{ cluster_recoveries.total }}**;成功率:**{{ "%.1f" | format(cluster_recoveries.success_rate * 100) }}%**

{% if cluster_recoveries.status_counts %}
**按状态**:
{% for kv in cluster_recoveries.status_counts | dictsort -%}
- {{ kv.0 }}:{{ kv.1 }}
{% endfor %}
{% endif %}
{% endif %}

---

*由 SRE 巡检图谱平台自动生成 — PRD-003 Sprint 2 cluster_overview*
