// specs/arrow/change_fact.rs
//
// ChangeEvent → Fact 映射 schema。PRD-002 ChangeEvent 模型对应。
// Phase 3 PRD-002 复刻时 engine-changes 直接消费此 schema。

// 字段           | 类型         | 来源
// ----------------|--------------|------------------------------------------
// id             | Utf8         | change_event_id
// change_type    | Utf8         | configmap_updated / secret_rotated /
//                |              | deployment_rolled / image_pushed
// source         | Utf8         | k8s_api / argo_cd / gitops / flagd / manual
// target_id      | Utf8         | 被变更的资源 ID
// target_type    | Utf8         | 资源类型
// actor          | Utf8         | 谁触发(用户 / pipeline / 系统)
// commit_sha     | Utf8         | git commit(可空)
// pipeline_url   | Utf8         | CI 链接(可空)
// yaml_diff      | Utf8         | unified diff 字符串(可空)
// diff_summary   | Utf8         | 单行摘要(可空)
// propagated_to  | List<Utf8>   | 反向 BFS 影响面 resource_id 列表
// severity       | Utf8         | low / medium / high
// timestamp      | Timestamp    | 事件发生时间
// observed_at    | Timestamp    | host 入库时间
