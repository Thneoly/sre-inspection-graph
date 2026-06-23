// specs/arrow/metric_fact.rs
//
// MetricSnapshot → Fact 映射 schema。Prometheus / spanmetrics 抓取的指标。

// 字段           | 类型         | 来源
// ----------------|--------------|------------------------------------------
// id             | Utf8         | snapshot_id
// resource_ref   | Utf8         | 关联资源 resource_id
// metric_name    | Utf8         | duration_p99 / cpu_usage / rps / ...
// metric_type    | Utf8         | gauge / counter / histogram(spanmetrics)
// value          | Float64      | 数值
// labels         | Utf8         | JSON 编码 label 集
// window_seconds | UInt32       | 聚合窗口(常 60 / 300 / 3600)
// timestamp      | Timestamp    | 数据点时间
// observed_at    | Timestamp    | host 入库时间
