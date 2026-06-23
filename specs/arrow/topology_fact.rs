// specs/arrow/topology_fact.rs
//
// Arrow Schema 定义 — 资源拓扑 Fact。
//
// 此文件作为 engine-core::facts::topology 模块的 source of truth,
// Phase 2 起 engine-core 直接 `include!` 此文件或重构为 path dep。
// Phase 1 仅作 schema 文档(不参与 cargo build)。
//
// 版本:0.1.0(见 specs/version.toml [arrow] 段)

// === 字段说明 ===
//
// TopologyFact 表示"某资源在某时刻的状态快照"。connector 产出后由 engine-core
// 写入 canonical store(Arrow + DataFusion + Parquet 历史)。
//
// 字段           | 类型         | 来源
// ----------------|--------------|--------------------------------------
// id             | Utf8         | UUID v4(connector 生成)
// source         | Utf8         | connector 名(如 "k8s-connector")
// resource_id    | Utf8         | "{type}:{cluster}:{namespace}:{name}"
// resource_type  | Utf8         | L1 14 类型之一(Pod / Deployment / ...)
// cluster_id     | Utf8         | 多集群路由用
// namespace      | Utf8         | k8s namespace(可空 — 集群级资源)
// name           | Utf8         | 资源短名
// health         | Utf8         | "green" / "yellow" / "red"(可空)
// attributes     | Utf8         | JSON 字符串(临时;Phase 3 切 Map<Utf8,Utf8>)
// timestamp      | Timestamp    | 微秒精度
// observed_at    | Timestamp    | host 入库时间(connector vs host 时钟差异调试用)
//
// === Rust 代码草案(Phase 2 进 engine-core::facts)===
//
// use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
// use std::sync::Arc;
//
// pub fn topology_fact_schema() -> Arc<Schema> {
//     Arc::new(Schema::new(vec![
//         Field::new("id", DataType::Utf8, false),
//         Field::new("source", DataType::Utf8, false),
//         Field::new("resource_id", DataType::Utf8, false),
//         Field::new("resource_type", DataType::Utf8, false),
//         Field::new("cluster_id", DataType::Utf8, true),
//         Field::new("namespace", DataType::Utf8, true),
//         Field::new("name", DataType::Utf8, false),
//         Field::new("health", DataType::Utf8, true),
//         Field::new("attributes", DataType::Utf8, true), // JSON,Phase 3 升 Map
//         Field::new(
//             "timestamp",
//             DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
//             false,
//         ),
//         Field::new(
//             "observed_at",
//             DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
//             false,
//         ),
//     ]))
// }
//
// === 兼容性 ===
//
// - 字段顺序与名字一旦定稿,Phase 2 内不变;新字段只能追加在末尾。
// - resource_id 与 reference 的 "{type}:{cluster}:{namespace}:{name}" 兼容
//   (PRD-001 cascade BFS / PRD-002 propagation BFS 直接用)。
// - JSON 属性是过渡;Map 升级时通过 schema_version bump 0.1 → 0.2 触发迁移。
