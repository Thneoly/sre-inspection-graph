//! SQLite storage backend.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use engine_core::Fact;
use engine_identity::{ChangeSet, ResolvedEdge, ResolvedNode, Topology};
use engine_recovery::{DryRunResult, RecoveryExecution, RecoveryStatus, VerifyStatus};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::{Storage, StorageError};

/// SQLite-backed local storage.
#[derive(Debug, Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Connect to a SQLite database file, creating it if needed.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the pool cannot be opened.
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        Self::connect_with_options(options).await
    }

    /// Connect to an in-memory SQLite database.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the pool cannot be opened.
    pub async fn connect_in_memory() -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::new().in_memory(true);
        Self::connect_with_options(options).await
    }

    async fn connect_with_options(options: SqliteConnectOptions) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    /// Run schema migrations required by this backend.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQLite rejects any migration statement.
    pub async fn migrate(&self) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS facts (
              id TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              source TEXT NOT NULL,
              resource_id TEXT NOT NULL,
              resource_type TEXT NOT NULL,
              timestamp INTEGER NOT NULL,
              attributes_json TEXT NOT NULL,
              inserted_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_facts_resource_id_timestamp
              ON facts(resource_id, timestamp DESC)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_facts_kind_timestamp
              ON facts(kind, timestamp DESC)
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_facts_source_timestamp
              ON facts(source, timestamp DESC)
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Phase 2.5 — materialized topology(Identity Resolver 的落地表)。
        // raw facts 是 append-only 真相源;这两张表是 resolve+diff 后的当前视图,
        // get_graph 读它(不再每次从 facts 重算)。
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS topology_nodes (
              resource_id TEXT PRIMARY KEY,
              resource_type TEXT NOT NULL,
              label TEXT NOT NULL,
              attributes_json TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS topology_edges (
              id TEXT PRIMARY KEY,
              source TEXT NOT NULL,
              target TEXT NOT NULL,
              edge_type TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Phase 3.2 - recovery_executions 表(RecoveryExecution 持久化)。
        // JSON 列(input_params / dry_run_result / result / verify_result)存序列化文本;
        // status / verify_status 存 snake_case 枚举文本。3.3 verifier/chain 复用同表。
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS recovery_executions (
              execution_id TEXT PRIMARY KEY,
              action_id TEXT NOT NULL,
              target_resource_id TEXT NOT NULL,
              target_resource_type TEXT NOT NULL,
              finding_id TEXT,
              input_params TEXT NOT NULL,
              dry_run_result TEXT NOT NULL,
              status TEXT NOT NULL,
              initiated_by TEXT NOT NULL,
              request_reason TEXT NOT NULL,
              initiated_at TEXT NOT NULL,
              executed_at TEXT NOT NULL,
              completed_at TEXT NOT NULL,
              result TEXT NOT NULL,
              rollback_execution_id TEXT,
              reverses_execution_id TEXT,
              cluster_id TEXT NOT NULL,
              verify_status TEXT NOT NULL,
              verify_result TEXT NOT NULL,
              verified_at TEXT NOT NULL,
              chain_id TEXT NOT NULL,
              chain_step_index INTEGER NOT NULL,
              approval_comment TEXT NOT NULL,
              approved_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_recovery_executions_status
              ON recovery_executions(status)
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Insert or update a batch of canonical facts.
    ///
    /// The raw fact table uses `Fact.id` as the primary key. Re-ingesting the
    /// same fact is idempotent; reusing an id with changed fields updates the
    /// row to match the latest ingestion.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQLite rejects any row.
    pub async fn upsert_facts(&self, facts: &[Fact]) -> Result<usize, StorageError> {
        let mut tx = self.pool.begin().await?;
        let inserted_at = now_unix_seconds()?;

        for fact in facts {
            sqlx::query(
                r#"
                INSERT INTO facts (
                    id,
                    kind,
                    source,
                    resource_id,
                    resource_type,
                    timestamp,
                    attributes_json,
                    inserted_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(id) DO UPDATE SET
                    kind = excluded.kind,
                    source = excluded.source,
                    resource_id = excluded.resource_id,
                    resource_type = excluded.resource_type,
                    timestamp = excluded.timestamp,
                    attributes_json = excluded.attributes_json,
                    inserted_at = excluded.inserted_at
                "#,
            )
            .bind(&fact.id)
            .bind(&fact.kind)
            .bind(&fact.source)
            .bind(&fact.resource_id)
            .bind(&fact.resource_type)
            .bind(
                i64::try_from(fact.timestamp).map_err(|_| StorageError::TimestampOutOfRange {
                    field: "timestamp",
                    value: fact.timestamp,
                })?,
            )
            .bind(&fact.attributes_json)
            .bind(inserted_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(facts.len())
    }

    /// Return the newest `topology-node` fact for each `resource_id`.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the query fails or a stored timestamp cannot
    /// be represented as `u64`.
    pub async fn latest_topology_facts(&self) -> Result<Vec<Fact>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT id, kind, source, resource_id, resource_type, timestamp, attributes_json
            FROM (
                SELECT
                    id,
                    kind,
                    source,
                    resource_id,
                    resource_type,
                    timestamp,
                    attributes_json,
                    ROW_NUMBER() OVER (
                        PARTITION BY resource_id
                        ORDER BY timestamp DESC, inserted_at DESC, id DESC
                    ) AS rn
                FROM facts
                WHERE kind = 'topology-node'
            )
            WHERE rn = 1
            ORDER BY resource_id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let timestamp: i64 = row.try_get("timestamp")?;
                let timestamp = u64::try_from(timestamp)
                    .map_err(|_| StorageError::NegativeTimestamp { value: timestamp })?;
                Ok(Fact {
                    id: row.try_get("id")?,
                    kind: row.try_get("kind")?,
                    source: row.try_get("source")?,
                    resource_id: row.try_get("resource_id")?,
                    resource_type: row.try_get("resource_type")?,
                    timestamp,
                    attributes_json: row.try_get("attributes_json")?,
                })
            })
            .collect()
    }

    /// Return the metric facts from the most recent metric sync.
    ///
    /// Prometheus connector emits all its `kind="metric"` facts at a single
    /// `timestamp` (one `clock::now_seconds()` call per sync). This returns every
    /// metric fact whose `timestamp` equals the max metric timestamp -- i.e. the
    /// latest sync's full metric set, used by `engine_identity::merge_metric_health`
    /// to overlay metric-derived health onto topology nodes.
    ///
    /// If no metric facts exist, returns an empty vec.
    ///
    /// # Errors
    /// Returns [`StorageError`] if the query fails or a stored timestamp cannot
    /// be represented as `u64`.
    pub async fn latest_metric_facts(&self) -> Result<Vec<Fact>, StorageError> {
        let rows = sqlx::query(
            r#"
            SELECT id, kind, source, resource_id, resource_type, timestamp, attributes_json
            FROM facts
            WHERE kind = 'metric'
              AND timestamp = (SELECT MAX(timestamp) FROM facts WHERE kind = 'metric')
            ORDER BY resource_id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let timestamp: i64 = row.try_get("timestamp")?;
                let timestamp = u64::try_from(timestamp)
                    .map_err(|_| StorageError::NegativeTimestamp { value: timestamp })?;
                Ok(Fact {
                    id: row.try_get("id")?,
                    kind: row.try_get("kind")?,
                    source: row.try_get("source")?,
                    resource_id: row.try_get("resource_id")?,
                    resource_type: row.try_get("resource_type")?,
                    timestamp,
                    attributes_json: row.try_get("attributes_json")?,
                })
            })
            .collect()
    }

    /// Read the current materialized [`Topology`] (resolved nodes + edges).
    ///
    /// This is the read source for the desktop graph view since Phase 2.5:
    /// `get_graph` reads here instead of re-deriving from raw facts.
    ///
    /// # Errors
    /// Returns [`StorageError`] if either query fails.
    pub async fn materialized_topology(&self) -> Result<Topology, StorageError> {
        let node_rows = sqlx::query(
            r#"
            SELECT resource_id, resource_type, label, attributes_json
            FROM topology_nodes
            ORDER BY resource_id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let nodes = node_rows
            .into_iter()
            .map(|row| {
                Ok(ResolvedNode {
                    resource_id: row.try_get("resource_id")?,
                    resource_type: row.try_get("resource_type")?,
                    label: row.try_get("label")?,
                    attributes_json: row.try_get("attributes_json")?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        let edge_rows = sqlx::query(
            r#"
            SELECT id, source, target, edge_type
            FROM topology_edges
            ORDER BY id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        let edges = edge_rows
            .into_iter()
            .map(|row| {
                Ok(ResolvedEdge {
                    id: row.try_get("id")?,
                    source: row.try_get("source")?,
                    target: row.try_get("target")?,
                    edge_type: row.try_get("edge_type")?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        Ok(Topology { nodes, edges })
    }

    /// Apply a [`ChangeSet`] to the materialized topology tables in one tx.
    ///
    /// UPSERTs changed nodes/edges and DELETEs removed ones — the minimal write
    /// computed by `engine_identity::diff`. Idempotent: re-applying an empty
    /// change set is a no-op commit.
    ///
    /// # Errors
    /// Returns [`StorageError`] if SQLite rejects any statement; the tx rolls
    /// back so the materialized view never lands half-applied.
    pub async fn apply_change_set(&self, change_set: &ChangeSet) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        let updated_at = now_unix_seconds()?;

        for node in &change_set.nodes_upserted {
            sqlx::query(
                r#"
                INSERT INTO topology_nodes (resource_id, resource_type, label, attributes_json, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(resource_id) DO UPDATE SET
                    resource_type = excluded.resource_type,
                    label = excluded.label,
                    attributes_json = excluded.attributes_json,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&node.resource_id)
            .bind(&node.resource_type)
            .bind(&node.label)
            .bind(&node.attributes_json)
            .bind(updated_at)
            .execute(&mut *tx)
            .await?;
        }
        for resource_id in &change_set.nodes_removed {
            sqlx::query("DELETE FROM topology_nodes WHERE resource_id = ?1")
                .bind(resource_id)
                .execute(&mut *tx)
                .await?;
        }

        for edge in &change_set.edges_upserted {
            sqlx::query(
                r#"
                INSERT INTO topology_edges (id, source, target, edge_type, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(id) DO UPDATE SET
                    source = excluded.source,
                    target = excluded.target,
                    edge_type = excluded.edge_type,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&edge.id)
            .bind(&edge.source)
            .bind(&edge.target)
            .bind(&edge.edge_type)
            .bind(updated_at)
            .execute(&mut *tx)
            .await?;
        }
        for id in &change_set.edges_removed {
            sqlx::query("DELETE FROM topology_edges WHERE id = ?1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Insert or update a [`RecoveryExecution`] (Phase 3.2)。
    ///
    /// 按 `execution_id` 幂等。JSON 字段(input_params / dry_run_result / result /
    /// verify_result)存序列化文本;status / verify_status 存 snake_case 枚举。
    pub async fn upsert_recovery_execution(
        &self,
        e: &RecoveryExecution,
    ) -> Result<(), StorageError> {
        let input_params = e.input_params.to_string();
        let dry_run_result = serde_json::to_string(&e.dry_run_result)
            .map_err(|e| StorageError::Clock(e.to_string()))?;
        let result = e.result.to_string();
        let verify_result = e.verify_result.to_string();
        let status = serde_json::to_string(&e.status).map_err(|e| StorageError::Clock(e.to_string()))?;
        let verify_status =
            serde_json::to_string(&e.verify_status).map_err(|e| StorageError::Clock(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO recovery_executions (
                execution_id, action_id, target_resource_id, target_resource_type,
                finding_id, input_params, dry_run_result, status,
                initiated_by, request_reason, initiated_at, executed_at, completed_at,
                result, rollback_execution_id, reverses_execution_id, cluster_id,
                verify_status, verify_result, verified_at, chain_id, chain_step_index,
                approval_comment, approved_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22, ?23, ?24
            )
            ON CONFLICT(execution_id) DO UPDATE SET
                action_id = excluded.action_id,
                target_resource_id = excluded.target_resource_id,
                target_resource_type = excluded.target_resource_type,
                finding_id = excluded.finding_id,
                input_params = excluded.input_params,
                dry_run_result = excluded.dry_run_result,
                status = excluded.status,
                initiated_by = excluded.initiated_by,
                request_reason = excluded.request_reason,
                initiated_at = excluded.initiated_at,
                executed_at = excluded.executed_at,
                completed_at = excluded.completed_at,
                result = excluded.result,
                rollback_execution_id = excluded.rollback_execution_id,
                reverses_execution_id = excluded.reverses_execution_id,
                cluster_id = excluded.cluster_id,
                verify_status = excluded.verify_status,
                verify_result = excluded.verify_result,
                verified_at = excluded.verified_at,
                chain_id = excluded.chain_id,
                chain_step_index = excluded.chain_step_index,
                approval_comment = excluded.approval_comment,
                approved_at = excluded.approved_at
            "#,
        )
        .bind(&e.execution_id)
        .bind(&e.action_id)
        .bind(&e.target_resource_id)
        .bind(&e.target_resource_type)
        .bind(&e.finding_id)
        .bind(&input_params)
        .bind(&dry_run_result)
        .bind(&status)
        .bind(&e.initiated_by)
        .bind(&e.request_reason)
        .bind(&e.initiated_at)
        .bind(&e.executed_at)
        .bind(&e.completed_at)
        .bind(&result)
        .bind(&e.rollback_execution_id)
        .bind(&e.reverses_execution_id)
        .bind(&e.cluster_id)
        .bind(&verify_status)
        .bind(&verify_result)
        .bind(&e.verified_at)
        .bind(&e.chain_id)
        .bind(e.chain_step_index)
        .bind(&e.approval_comment)
        .bind(&e.approved_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 取单个 [`RecoveryExecution`];不存在返 None。
    pub async fn get_recovery_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<RecoveryExecution>, StorageError> {
        let row = sqlx::query(
            r#"SELECT * FROM recovery_executions WHERE execution_id = ?1"#,
        )
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_execution).transpose()
    }

    /// 列 [`RecoveryExecution`](新到旧,按 initiated_at 降序)。
    pub async fn list_recovery_executions(
        &self,
        limit: i64,
    ) -> Result<Vec<RecoveryExecution>, StorageError> {
        let rows = sqlx::query(
            r#"SELECT * FROM recovery_executions ORDER BY initiated_at DESC LIMIT ?1"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_execution).collect()
    }
}

/// 把一行 recovery_executions 解析成 [`RecoveryExecution`]。
fn row_to_execution(row: sqlx::sqlite::SqliteRow) -> Result<RecoveryExecution, StorageError> {
    let input_params: String = row.try_get("input_params")?;
    let dry_run_result: String = row.try_get("dry_run_result")?;
    let result: String = row.try_get("result")?;
    let verify_result: String = row.try_get("verify_result")?;
    let status: String = row.try_get("status")?;
    let verify_status: String = row.try_get("verify_status")?;
    Ok(RecoveryExecution {
        execution_id: row.try_get("execution_id")?,
        action_id: row.try_get("action_id")?,
        target_resource_id: row.try_get("target_resource_id")?,
        target_resource_type: row.try_get("target_resource_type")?,
        finding_id: row.try_get("finding_id")?,
        input_params: serde_json::from_str(&input_params)
            .map_err(|e| StorageError::Clock(format!("input_params parse: {e}")))?,
        dry_run_result: serde_json::from_str::<DryRunResult>(&dry_run_result)
            .map_err(|e| StorageError::Clock(format!("dry_run_result parse: {e}")))?,
        status: serde_json::from_str::<RecoveryStatus>(&status)
            .map_err(|e| StorageError::Clock(format!("status parse: {e}")))?,
        initiated_by: row.try_get("initiated_by")?,
        request_reason: row.try_get("request_reason")?,
        initiated_at: row.try_get("initiated_at")?,
        executed_at: row.try_get("executed_at")?,
        completed_at: row.try_get("completed_at")?,
        result: serde_json::from_str(&result)
            .map_err(|e| StorageError::Clock(format!("result parse: {e}")))?,
        rollback_execution_id: row.try_get("rollback_execution_id")?,
        reverses_execution_id: row.try_get("reverses_execution_id")?,
        cluster_id: row.try_get("cluster_id")?,
        verify_status: serde_json::from_str::<VerifyStatus>(&verify_status)
            .map_err(|e| StorageError::Clock(format!("verify_status parse: {e}")))?,
        verify_result: serde_json::from_str(&verify_result)
            .map_err(|e| StorageError::Clock(format!("verify_result parse: {e}")))?,
        verified_at: row.try_get("verified_at")?,
        chain_id: row.try_get("chain_id")?,
        chain_step_index: row.try_get("chain_step_index")?,
        approval_comment: row.try_get("approval_comment")?,
        approved_at: row.try_get("approved_at")?,
    })
}

impl Storage for SqliteStorage {
    fn backend_name(&self) -> &'static str {
        "sqlite"
    }
}

fn now_unix_seconds() -> Result<i64, StorageError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| StorageError::Clock(e.to_string()))?;
    i64::try_from(duration.as_secs()).map_err(|_| StorageError::TimestampOutOfRange {
        field: "inserted_at",
        value: duration.as_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(
        id: &str,
        resource_id: &str,
        resource_type: &str,
        timestamp: u64,
        attributes_json: &str,
    ) -> Fact {
        Fact::new(
            id,
            "topology-node",
            "test",
            resource_id,
            resource_type,
            timestamp,
            attributes_json,
        )
    }

    async fn migrated_store() -> SqliteStorage {
        let store = SqliteStorage::connect_in_memory()
            .await
            .expect("connect in-memory sqlite");
        store.migrate().await.expect("migrate schema");
        store
    }

    #[tokio::test]
    async fn migrate_creates_schema() {
        let store = migrated_store().await;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'facts'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("query sqlite_master");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn upsert_facts_inserts_rows() {
        let store = migrated_store().await;
        let facts = vec![
            fact("cluster", "cluster:demo", "Cluster", 1, "{}"),
            fact(
                "ns",
                "ns:demo:default",
                "Namespace",
                2,
                r#"{"parent_resource_id":"cluster:demo"}"#,
            ),
        ];

        let written = store.upsert_facts(&facts).await.expect("upsert facts");
        assert_eq!(written, 2);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM facts")
            .fetch_one(&store.pool)
            .await
            .expect("count facts");
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn upsert_same_id_updates_row() {
        let store = migrated_store().await;
        store
            .upsert_facts(&[fact("same", "pod:old", "Pod", 1, "{}")])
            .await
            .expect("insert first row");
        store
            .upsert_facts(&[fact("same", "pod:new", "Pod", 2, r#"{"v":2}"#)])
            .await
            .expect("update same id");

        let row = sqlx::query(
            "SELECT resource_id, timestamp, attributes_json FROM facts WHERE id = 'same'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("fetch updated row");
        let resource_id: String = row.try_get("resource_id").expect("resource_id");
        let timestamp: i64 = row.try_get("timestamp").expect("timestamp");
        let attributes_json: String = row.try_get("attributes_json").expect("attributes_json");
        assert_eq!(resource_id, "pod:new");
        assert_eq!(timestamp, 2);
        assert_eq!(attributes_json, r#"{"v":2}"#);
    }

    #[tokio::test]
    async fn latest_topology_facts_returns_newest_per_resource() {
        let store = migrated_store().await;
        store
            .upsert_facts(&[
                fact(
                    "pod-old",
                    "pod:demo:default:web-0",
                    "Pod",
                    10,
                    r#"{"old":true}"#,
                ),
                fact("cluster", "cluster:demo", "Cluster", 1, "{}"),
                fact(
                    "pod-new",
                    "pod:demo:default:web-0",
                    "Pod",
                    20,
                    r#"{"new":true}"#,
                ),
                Fact::new(
                    "metric",
                    "metric",
                    "test",
                    "pod:demo:default:web-0",
                    "Pod",
                    30,
                    "{}",
                ),
            ])
            .await
            .expect("upsert facts");

        let latest = store
            .latest_topology_facts()
            .await
            .expect("query latest topology facts");
        let ids: Vec<&str> = latest.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["cluster", "pod-new"]);
        assert_eq!(latest[1].attributes_json, r#"{"new":true}"#);
    }

    #[tokio::test]
    async fn malformed_attributes_json_does_not_block_storage() {
        let store = migrated_store().await;
        store
            .upsert_facts(&[fact(
                "bad-json",
                "service:demo:default:web",
                "Service",
                1,
                "not-json",
            )])
            .await
            .expect("upsert malformed json as opaque text");

        let latest = store
            .latest_topology_facts()
            .await
            .expect("query latest topology facts");
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].attributes_json, "not-json");
    }

    #[tokio::test]
    async fn latest_metric_facts_returns_only_latest_sync() {
        let store = migrated_store().await;
        // 第一次 metric sync(ts=10):两条 metric fact
        store
            .upsert_facts(&[
                Fact::new(
                    "m1",
                    "metric",
                    "prometheus",
                    "svc:a",
                    "Service",
                    10,
                    r#"{"metric":"span_p99_ms","value":400.0}"#,
                ),
                Fact::new(
                    "m2",
                    "metric",
                    "prometheus",
                    "svc:b",
                    "Service",
                    10,
                    r#"{"metric":"span_error_rate_pct","value":2.0}"#,
                ),
            ])
            .await
            .expect("upsert first metric sync");
        // 第二次 metric sync(ts=20):只 svc:a 有新值(svc:b 这轮没数据)
        store
            .upsert_facts(&[Fact::new(
                "m3",
                "metric",
                "prometheus",
                "svc:a",
                "Service",
                20,
                r#"{"metric":"span_p99_ms","value":1200.0}"#,
            )])
            .await
            .expect("upsert second metric sync");
        // 一条 topology-node fact(ts=30)不应影响 metric 查询
        store
            .upsert_facts(&[fact("t1", "svc:a", "Service", 30, r#"{}"#)])
            .await
            .expect("upsert topology fact");

        let latest = store
            .latest_metric_facts()
            .await
            .expect("query latest metric facts");
        // 只回最新 metric sync(ts=20)的 fact -> 仅 m3
        let ids: Vec<&str> = latest.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, vec!["m3"]);
        assert_eq!(latest[0].timestamp, 20);
    }

    #[tokio::test]
    async fn latest_metric_facts_empty_when_no_metrics() {
        let store = migrated_store().await;
        // 只有 topology-node fact -> metric 查询返空
        store
            .upsert_facts(&[fact("c", "cluster:demo", "Cluster", 1, "{}")])
            .await
            .expect("upsert topology fact");
        let latest = store
            .latest_metric_facts()
            .await
            .expect("query latest metric facts");
        assert!(latest.is_empty());
    }

    #[tokio::test]
    async fn merge_metric_health_flows_through_resolve_diff_apply() {
        // 验证 sync_all_now 的 part1 接线 seam:resolve -> merge_metric_health
        // -> diff -> apply,materialized 拓扑反映合并后的 health(prometheus warning
        // > k8s normal -> warning)。覆盖 merge 输出喂给 diff/apply 的衔接(单测只测
        // merge 本身,这条测整条 pipeline)。
        let store = migrated_store().await;
        // service 节点:k8s 说 normal/low
        store
            .upsert_facts(&[fact(
                "svc1",
                "service:c:ns:cart",
                "Service",
                100,
                r#"{"health_status":"normal","risk_level":"low"}"#,
            )])
            .await
            .expect("upsert topology fact");
        // 同 resource_id 的 metric fact:prometheus error_rate=3.0 -> warning
        store
            .upsert_facts(&[Fact::new(
                "m1",
                "metric",
                "prometheus",
                "service:c:ns:cart",
                "Service",
                200,
                r#"{"metric":"span_error_rate_pct","value":3.0}"#,
            )])
            .await
            .expect("upsert metric fact");

        let topo_facts = store
            .latest_topology_facts()
            .await
            .expect("latest topology facts");
        let metric_facts = store
            .latest_metric_facts()
            .await
            .expect("latest metric facts");
        assert_eq!(metric_facts.len(), 1);

        // resolve -> merge(prometheus warning > k8s normal -> warning)
        let mut next = engine_identity::resolve(&topo_facts);
        next = engine_identity::merge_metric_health(
            &next,
            &metric_facts,
            &engine_identity::HealthThresholds::default(),
        );
        assert!(
            next.nodes[0]
                .attributes_json
                .contains(r#""health_status":"warning""#),
            "merged health should be warning: {}",
            next.nodes[0].attributes_json
        );

        // diff(空 materialized)-> apply
        let current = store
            .materialized_topology()
            .await
            .expect("read empty materialized");
        assert!(current.is_empty());
        let cs = engine_identity::diff(&current, &next);
        assert_eq!(cs.summary().nodes_upserted, 1);
        store
            .apply_change_set(&cs)
            .await
            .expect("apply change set");

        // materialized 反映合并 health;topology_to_graph summary 也反映
        let mat = store
            .materialized_topology()
            .await
            .expect("read materialized");
        assert_eq!(mat.nodes.len(), 1);
        assert!(mat.nodes[0]
            .attributes_json
            .contains(r#""health_status":"warning""#));
        assert!(mat.nodes[0]
            .attributes_json
            .contains(r#""risk_level":"medium""#));
        let g = engine_identity::topology_to_graph(&mat);
        assert_eq!(g.summary.health_counts["warning"], 1);
        assert_eq!(g.summary.health_counts["normal"], 0);
    }

    #[tokio::test]
    async fn recovery_execution_round_trips_sqlite() {
        let store = migrated_store().await;
        // 最小拓扑(1 Deployment,desired_replicas=3)
        let mut topo = Topology {
            nodes: vec![ResolvedNode {
                resource_id: "deploy:c:ns:app".into(),
                resource_type: "Deployment".into(),
                label: "app".into(),
                attributes_json: r#"{"desired_replicas":3}"#.into(),
            }],
            edges: vec![],
        };
        let mut reg = engine_recovery::ExecutionRegistry::new();
        let e = engine_recovery::execute(
            &mut reg,
            "scale_deployment",
            "deploy:c:ns:app",
            &serde_json::json!({ "replicas_delta": 2 }),
            &mut topo,
            "tester",
            "test",
        )
        .expect("execute");
        assert_eq!(e.status, engine_recovery::RecoveryStatus::Succeeded);
        assert_eq!(e.result["new_replicas"], 5);

        store
            .upsert_recovery_execution(&e)
            .await
            .expect("upsert");
        let got = store
            .get_recovery_execution(&e.execution_id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(got.execution_id, e.execution_id);
        assert_eq!(got.status, e.status);
        assert_eq!(got.result["new_replicas"], 5);
        assert_eq!(got.dry_run_result.action_id, "scale_deployment");
        assert_eq!(got.cluster_id, "c"); // target_id 第二段

        let listed = store.list_recovery_executions(10).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].execution_id, e.execution_id);

        // upsert 幂等:同 id 再写不报错,不新增行
        store
            .upsert_recovery_execution(&e)
            .await
            .expect("upsert again");
        let listed2 = store.list_recovery_executions(10).await.expect("list2");
        assert_eq!(listed2.len(), 1);
    }

    #[tokio::test]
    async fn materialized_topology_round_trips_resolve_diff_apply() {
        let store = migrated_store().await;
        // 第一次 sync:cluster → ns → pod
        store
            .upsert_facts(&[
                fact("c", "cluster:demo", "Cluster", 1, "{}"),
                fact(
                    "n",
                    "ns:demo:default",
                    "Namespace",
                    2,
                    r#"{"parent_resource_id":"cluster:demo"}"#,
                ),
                fact(
                    "p",
                    "pod:demo:default:web-0",
                    "Pod",
                    3,
                    r#"{"parent_resource_id":"ns:demo:default"}"#,
                ),
            ])
            .await
            .expect("upsert facts");

        // resolve(latest facts) → diff(materialized=empty) → apply
        let facts = store.latest_topology_facts().await.expect("latest facts");
        let next = engine_identity::resolve(&facts);
        let current = store.materialized_topology().await.expect("read empty");
        assert!(current.is_empty());
        let cs = engine_identity::diff(&current, &next);
        assert_eq!(cs.summary().nodes_upserted, 3);
        assert_eq!(cs.summary().edges_upserted, 2);
        store.apply_change_set(&cs).await.expect("apply");

        // materialized 现与 resolve 结果一致
        let materialized = store.materialized_topology().await.expect("read materialized");
        assert_eq!(materialized, next);

        // 二次 sync:pod 消失,ns 属性变 → diff 只动 ns(upsert)+ pod(remove)
        store
            .upsert_facts(&[fact(
                "n2",
                "ns:demo:default",
                "Namespace",
                10,
                r#"{"parent_resource_id":"cluster:demo","risk_level":"high"}"#,
            )])
            .await
            .expect("upsert ns update");
        // 模拟 pod 被删:这里用一条新 cluster-only 视图不现实,改为直接构造 next2
        // 真实链路 pod 不再出现在 facts 即消失;此处验证 diff/apply 的 remove 分支:
        let next2 = engine_identity::Topology {
            nodes: materialized
                .nodes
                .iter()
                .filter(|n| n.resource_id != "pod:demo:default:web-0")
                .cloned()
                .map(|mut n| {
                    if n.resource_id == "ns:demo:default" {
                        n.attributes_json =
                            r#"{"parent_resource_id":"cluster:demo","risk_level":"high"}"#.into();
                    }
                    n
                })
                .collect(),
            edges: materialized
                .edges
                .iter()
                .filter(|e| e.target != "pod:demo:default:web-0")
                .cloned()
                .collect(),
        };
        let cs2 = engine_identity::diff(&materialized, &next2);
        assert_eq!(cs2.nodes_removed, vec!["pod:demo:default:web-0"]);
        assert_eq!(cs2.edges_removed, vec!["ns:demo:default->pod:demo:default:web-0"]);
        assert_eq!(cs2.summary().nodes_upserted, 1); // 仅 ns
        store.apply_change_set(&cs2).await.expect("apply cs2");

        let after = store.materialized_topology().await.expect("read after");
        assert_eq!(after, next2);
        assert_eq!(after.nodes.len(), 2); // cluster + ns
        assert_eq!(after.edges.len(), 1); // cluster->ns
    }
}
