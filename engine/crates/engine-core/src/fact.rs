//! Canonical Fact —— WIT `sre:inspection/connector.fact` 的 Rust 镜像 +
//! Arrow `RecordBatch` 转换。
//!
//! 7 列 schema(全 non-null,与 WIT record 完全一致):
//!
//! | 列名           | DataType  | 来源                                 |
//! |---------------|-----------|--------------------------------------|
//! | id            | Utf8      | guest 端生成的全局唯一 ID            |
//! | kind          | Utf8      | topology-node / metric / change / .. |
//! | source        | Utf8      | connector 名(`hello-world` 等)     |
//! | resource_id   | Utf8      | DSS resource_id(`comp:cluster:..`)  |
//! | resource_type | Utf8      | L1 14 类型 PascalCase 字符串         |
//! | timestamp     | UInt64    | Unix epoch 秒                        |
//! | attributes    | Utf8      | JSON 字符串(灵活段,guest 自定义)  |
//!
//! attributes 不展开 struct/map —— guest 自定义字段太散,JSON 走 Arrow column 即够,
//! 真要做高效查询走 DataFusion `json_extract`。

use std::sync::Arc;

use arrow::array::{ArrayRef, StringArray, UInt64Array};
use arrow::record_batch::RecordBatch;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use serde::{Deserialize, Serialize};

/// 单条 Fact —— 与 WIT `sre:inspection/connector.fact` record 完全一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    /// 全局唯一 ID(由 connector 端产生)。
    pub id: String,
    /// Fact 类型:topology-node / topology-edge / metric / change / alert。
    pub kind: String,
    /// 数据源 —— connector 名,与 manifest.toml `name` 对齐。
    pub source: String,
    /// 资源 ID —— DSS resource_id 同款命名空间(`comp:cluster:ns:name`)。
    pub resource_id: String,
    /// 资源类型 —— L1 14 类型 PascalCase 名。
    pub resource_type: String,
    /// 时间戳(Unix epoch 秒)。
    pub timestamp: u64,
    /// JSON 编码的属性段(灵活,connector 自定义)。
    pub attributes_json: String,
}

impl Fact {
    /// 简便构造器 —— 主要给测试 / 适配层用。
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        source: impl Into<String>,
        resource_id: impl Into<String>,
        resource_type: impl Into<String>,
        timestamp: u64,
        attributes_json: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            source: source.into(),
            resource_id: resource_id.into(),
            resource_type: resource_type.into(),
            timestamp,
            attributes_json: attributes_json.into(),
        }
    }
}

/// 7 列 Arrow `Schema` —— 所有 Fact RecordBatch 共用。
///
/// Phase 2 起 engine-storage 的 parquet backend 用此 schema 写文件,
/// engine-cli 的 Arrow Flight server 也基于此对外暴露查询接口。
pub fn fact_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("resource_id", DataType::Utf8, false),
        Field::new("resource_type", DataType::Utf8, false),
        Field::new("timestamp", DataType::UInt64, false),
        Field::new("attributes_json", DataType::Utf8, false),
    ]))
}

/// Fact 批 —— `Vec<Fact>` + 一次性转 Arrow `RecordBatch` 的容器。
///
/// 不持 RecordBatch(转换是惰性的、调 `to_record_batch` 时才组列),这样
/// 调用方可以 `extend(more_facts)` 攒批后再一次性转储,避免每条都重建 Array。
#[derive(Debug, Clone, Default)]
pub struct FactBatch {
    facts: Vec<Fact>,
}

impl FactBatch {
    /// 创建空批。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从已有 `Vec<Fact>` 构造。
    pub fn from_vec(facts: Vec<Fact>) -> Self {
        Self { facts }
    }

    /// 追加单条 fact。
    pub fn push(&mut self, fact: Fact) {
        self.facts.push(fact);
    }

    /// 追加多条 fact。
    pub fn extend(&mut self, facts: impl IntoIterator<Item = Fact>) {
        self.facts.extend(facts);
    }

    /// 当前批 fact 数。
    pub fn len(&self) -> usize {
        self.facts.len()
    }

    /// 批是否为空。
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// 借引用看内部 Fact 列表(不消费)。
    pub fn as_slice(&self) -> &[Fact] {
        &self.facts
    }

    /// 消费并拿走内部 Fact 列表。
    pub fn into_vec(self) -> Vec<Fact> {
        self.facts
    }

    /// 转 Arrow `RecordBatch`。空批返长度为 0 的 batch(schema 仍正确)。
    ///
    /// # Errors
    /// 仅在 Arrow 内部 schema-array 长度不一致时返错 —— 我们这里从 `Vec<Fact>`
    /// 同时构所有列,正常路径不会失败。
    pub fn to_record_batch(&self) -> Result<RecordBatch, FactError> {
        let schema = fact_schema();
        let n = self.facts.len();

        // 同步建 7 个列 —— 都按 fact order 严格对应行。
        let id: ArrayRef = Arc::new(StringArray::from_iter_values(
            self.facts.iter().map(|f| f.id.as_str()),
        ));
        let kind: ArrayRef = Arc::new(StringArray::from_iter_values(
            self.facts.iter().map(|f| f.kind.as_str()),
        ));
        let source: ArrayRef = Arc::new(StringArray::from_iter_values(
            self.facts.iter().map(|f| f.source.as_str()),
        ));
        let resource_id: ArrayRef = Arc::new(StringArray::from_iter_values(
            self.facts.iter().map(|f| f.resource_id.as_str()),
        ));
        let resource_type: ArrayRef = Arc::new(StringArray::from_iter_values(
            self.facts.iter().map(|f| f.resource_type.as_str()),
        ));
        let timestamp: ArrayRef = Arc::new(UInt64Array::from_iter_values(
            self.facts.iter().map(|f| f.timestamp),
        ));
        let attributes: ArrayRef = Arc::new(StringArray::from_iter_values(
            self.facts.iter().map(|f| f.attributes_json.as_str()),
        ));

        let columns = vec![
            id,
            kind,
            source,
            resource_id,
            resource_type,
            timestamp,
            attributes,
        ];

        // 列数一致是手工拼,长度一致由 from_iter_values + 同一个 self.facts 保证。
        let batch = RecordBatch::try_new(schema, columns).map_err(|e| {
            FactError::ArrowBatch(format!("build RecordBatch (n={n}): {e}"))
        })?;
        Ok(batch)
    }
}

impl From<Vec<Fact>> for FactBatch {
    fn from(facts: Vec<Fact>) -> Self {
        Self::from_vec(facts)
    }
}

impl FromIterator<Fact> for FactBatch {
    fn from_iter<T: IntoIterator<Item = Fact>>(iter: T) -> Self {
        Self::from_vec(iter.into_iter().collect())
    }
}

/// engine-core 的 Fact 层错误。
#[derive(Debug, thiserror::Error)]
pub enum FactError {
    /// Arrow RecordBatch 构造失败 —— 实践上几乎不会触发,留给极端 schema 不一致用。
    #[error("arrow batch error: {0}")]
    ArrowBatch(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, StringArray, UInt64Array};

    fn sample_fact(id: &str, ts: u64) -> Fact {
        Fact::new(
            id,
            "topology-node",
            "hello-world",
            "demo:placeholder:default:hello",
            "Placeholder",
            ts,
            r#"{"greeting":"hello, world"}"#,
        )
    }

    #[test]
    fn schema_has_seven_columns() {
        let s = fact_schema();
        let names: Vec<&str> = s.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            vec![
                "id",
                "kind",
                "source",
                "resource_id",
                "resource_type",
                "timestamp",
                "attributes_json",
            ]
        );
        // timestamp 是 UInt64,其余 Utf8
        assert_eq!(s.field(5).data_type(), &DataType::UInt64);
        assert_eq!(s.field(0).data_type(), &DataType::Utf8);
        assert_eq!(s.field(6).data_type(), &DataType::Utf8);
        // 全 non-null
        for f in s.fields() {
            assert!(!f.is_nullable(), "field {} should be non-nullable", f.name());
        }
    }

    #[test]
    fn empty_batch_roundtrips() {
        let b = FactBatch::new();
        assert!(b.is_empty());
        assert_eq!(b.len(), 0);
        let rb = b.to_record_batch().expect("empty batch ok");
        assert_eq!(rb.num_rows(), 0);
        assert_eq!(rb.num_columns(), 7);
    }

    #[test]
    fn batch_from_vec_emits_correct_arrays() {
        let facts = vec![
            sample_fact("a", 100),
            sample_fact("b", 200),
            sample_fact("c", 300),
        ];
        let b = FactBatch::from_vec(facts);
        let rb = b.to_record_batch().expect("batch ok");
        assert_eq!(rb.num_rows(), 3);

        let ids = rb
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("id column is Utf8");
        assert_eq!(ids.value(0), "a");
        assert_eq!(ids.value(1), "b");
        assert_eq!(ids.value(2), "c");

        let ts = rb
            .column(5)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .expect("timestamp column is UInt64");
        assert_eq!(ts.value(0), 100);
        assert_eq!(ts.value(1), 200);
        assert_eq!(ts.value(2), 300);
    }

    #[test]
    fn fact_serde_roundtrips() {
        let f = sample_fact("x", 42);
        let s = serde_json::to_string(&f).unwrap();
        let back: Fact = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn extend_pushes_into_same_batch() {
        let mut b = FactBatch::new();
        b.push(sample_fact("a", 1));
        b.extend([sample_fact("b", 2), sample_fact("c", 3)]);
        assert_eq!(b.len(), 3);
    }

    #[test]
    fn from_iter_collects() {
        let b: FactBatch = (0..5u64).map(|i| sample_fact(&format!("f{i}"), i)).collect();
        assert_eq!(b.len(), 5);
        let rb = b.to_record_batch().unwrap();
        assert_eq!(rb.num_rows(), 5);
    }
}
