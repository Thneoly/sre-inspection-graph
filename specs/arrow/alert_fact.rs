// specs/arrow/alert_fact.rs
//
// AlertEvent → Fact 映射 schema。PRD-004 Phase 2 AlertEvent 模型对应。

// 字段           | 类型         | 来源
// ----------------|--------------|------------------------------------------
// id             | Utf8         | alert_event_id
// rule_id        | Utf8         | AlertRule.rule_id
// resource_ref   | Utf8         | 被告警的 resource_id
// severity       | Utf8         | warning / critical
// status         | Utf8         | firing / resolved
// metric         | Utf8         | 触发的指标名(如 "duration_p99")
// actual         | Float64      | 实测值(可空)
// expected       | Float64      | 阈值(可空)
// description    | Utf8         | 告警描述
// fired_at       | Timestamp    | 首次 firing 时间
// resolved_at    | Timestamp    | 解除时间(可空)
// observed_at    | Timestamp    | host 入库时间
