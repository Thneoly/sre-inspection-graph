# 事件报告 - {{ anchor_id }}

- **模板**:incident_report
- **生成时间**:{{ generated_at }}
- **报告 ID**:{{ report_id }}
- **锚点**:{{ anchor_id }}({{ anchor_kind_label }})

---

{% if modules.incident_summary %}
## 1. 事件摘要

- **类型**:{{ incident_summary.kind_label }}
- **锚点 ID**:`{{ incident_summary.anchor_id }}`
- **目标**:`{{ incident_summary.target_id }}` ({{ incident_summary.target_type | default(value="Unknown") }})
- **时间**:{{ incident_summary.timestamp | default(value="-") }}
- **描述**:{{ incident_summary.description | default(value="-") }}
- **严重度**:{{ incident_summary.severity | default(value="-") }}
- **受影响节点总数**:**{{ incident_summary.affected_total }}**

{% if incident_summary.affected_by_type %}
**受影响类型分布**:
{% for kv in incident_summary.affected_by_type -%}
- {{ kv.type_name }}:{{ kv.count }}
{% endfor %}
{% endif %}

{% if incident_summary.affected_nodes %}
### 受影响节点

| 资源 | 类型 | 名称 |
|---|---|---|
{% for n in incident_summary.affected_nodes -%}
| `{{ n.resource_id }}` | {{ n.resource_type }} | {{ n.name }} |
{% endfor %}
{% else %}
未发现受影响节点。
{% endif %}
{% endif %}

{% if modules.incident_timeline %}
## 2. 事件时间线(±{{ incident_timeline.window_seconds }} 秒窗口)

锚点时间:**{{ incident_timeline.anchor_timestamp | default(value="-") }}**;事件数:**{{ incident_timeline.total }}**

{% if incident_timeline.events %}
| 时间 | 类型 | 动作/变更 | 目标 | 操作者 | 状态 |
|---|---|---|---|---|---|
{% for it in incident_timeline.events -%}
| {{ it.timestamp | default(value="-") }} | {{ it.kind_label }} | {{ it.type }} | `{{ it.target_id }}` | {{ it.actor | default(value="-") }} | {{ it.severity | default(value="-") }} |
{% endfor %}
{% else %}
窗口内无关联事件。
{% endif %}
{% endif %}

{% if modules.incident_recoveries %}
## 3. 已执行恢复 & 推荐后续

### 已执行
{% if incident_recoveries.executed %}
| 执行 ID | 动作 | 目标 | 状态 | 发起者 | 起 | 止 |
|---|---|---|---|---|---|---|
{% for e in incident_recoveries.executed -%}
| `{{ e.execution_id_short }}` | {{ e.action_id }} | `{{ e.target_id }}` | {{ e.status }} | {{ e.initiated_by | default(value="-") }} | {{ e.initiated_at | default(value="-") }} | {{ e.completed_at | default(value="-") }} |
{% endfor %}
{% else %}
当前事件尚未执行恢复动作。
{% endif %}

### 推荐后续
{% if incident_recoveries.recommended %}
{% for a in incident_recoveries.recommended -%}
{{ loop.index }}. **{{ a.action_id }}** -> `{{ a.target_id }}` - {{ a.rationale }}
{% endfor %}
{% else %}
当前无额外推荐。
{% endif %}

> 动作执行请到「审批中心 / 恢复历史」发起(关联 PRD-001)。
{% endif %}

---

*由 SRE 巡检图谱平台自动生成 - PRD-003 Phase 4.2b incident_report(Markdown)*
