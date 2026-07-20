//! SQLite storage backend.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use engine_core::Fact;
use engine_identity::{ChangeSet, ResolvedEdge, ResolvedNode, Topology};
use engine_changes::{AlertEvent, ChangeEvent};
use engine_recovery::{
    DryRunResult, RecoveryChain, RecoveryExecution, RecoveryStatus, VerifyStatus,
};
use engine_reports::ReportSubscription;
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

        // Phase 3.6 - change_events 表(ChangeEvent 持久化)。enum 列(change_type /
        // source / severity_estimate)存 snake_case JSON 文本;diff_summary / propagated_to
        // 存 JSON 文本。ChangeRegistry 启动从本表载入,record_change 后 upsert。
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS change_events (
              change_event_id TEXT PRIMARY KEY,
              change_type TEXT NOT NULL,
              target_resource_id TEXT NOT NULL,
              target_resource_type TEXT NOT NULL,
              changed_at TEXT NOT NULL,
              changed_by TEXT NOT NULL,
              source TEXT NOT NULL,
              description TEXT NOT NULL,
              diff_summary TEXT NOT NULL,
              related_commit TEXT NOT NULL,
              related_pr TEXT NOT NULL,
              severity_estimate TEXT NOT NULL,
              propagated_to TEXT NOT NULL,
              commit_sha TEXT NOT NULL,
              pipeline_url TEXT NOT NULL,
              git_repo TEXT NOT NULL,
              cluster_id TEXT NOT NULL,
              yaml_diff TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_change_events_changed_at
              ON change_events(changed_at DESC)
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Phase 3.6 - recovery_chains 表(RecoveryChain 持久化)。enum 列(status /
        // on_failure)存 snake_case JSON 文本;step_executions 存 JSON 数组文本。
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS recovery_chains (
              chain_id TEXT PRIMARY KEY,
              template_id TEXT NOT NULL,
              target_resource_id TEXT NOT NULL,
              status TEXT NOT NULL,
              on_failure TEXT NOT NULL,
              step_executions TEXT NOT NULL,
              current_step_index INTEGER NOT NULL,
              total_steps INTEGER NOT NULL,
              initiated_by TEXT NOT NULL,
              request_reason TEXT NOT NULL,
              initiated_at TEXT NOT NULL,
              completed_at TEXT NOT NULL,
              approval_id TEXT NOT NULL,
              failure_reason TEXT NOT NULL,
              template_name TEXT NOT NULL,
              approval_comment TEXT NOT NULL,
              approved_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Phase 3.6 - alert_events 表(AlertEvent 持久化)。enum 列(severity / status)
        // 存 snake_case JSON 文本。无 live 源(k8s-watch/webhook 延后);仅 record_alert 手动录。
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS alert_events (
              alert_event_id TEXT PRIMARY KEY,
              alert_name TEXT NOT NULL,
              severity TEXT NOT NULL,
              status TEXT NOT NULL,
              fired_at TEXT NOT NULL,
              resource_ref TEXT NOT NULL,
              rule_id TEXT NOT NULL,
              metric_name TEXT NOT NULL,
              metric_value REAL NOT NULL,
              summary TEXT NOT NULL,
              description TEXT NOT NULL,
              cluster_id TEXT NOT NULL,
              resolved_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Phase 4.3 - report_subscriptions 表(ReportSubscription 持久化;调度配置不能丢)。
        // enum 列(template_id / last_status)存 snake_case JSON 文本;scope/modules/recipients
        // 存 JSON 文本(对齐 3.6 模式)。
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS report_subscriptions (
              subscription_id TEXT PRIMARY KEY,
              template_id TEXT NOT NULL,
              scope TEXT NOT NULL,
              modules TEXT NOT NULL,
              cron TEXT NOT NULL,
              recipients TEXT NOT NULL,
              enabled INTEGER NOT NULL,
              created_at TEXT NOT NULL,
              last_run_at TEXT NOT NULL,
              last_status TEXT NOT NULL,
              last_error TEXT NOT NULL,
              last_report_id TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_report_subscriptions_created_at
              ON report_subscriptions(created_at DESC)
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
                WHERE kind IN ('topology-node', 'topology-edge')
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

    // ===== Phase 3.6: change_events =====

    /// Insert or update a [`ChangeEvent`]。按 `change_event_id` 幂等。
    pub async fn upsert_change_event(&self, e: &ChangeEvent) -> Result<(), StorageError> {
        let change_type = serde_json::to_string(&e.change_type).map_err(|e| StorageError::Clock(e.to_string()))?;
        let source = serde_json::to_string(&e.source).map_err(|e| StorageError::Clock(e.to_string()))?;
        let severity =
            serde_json::to_string(&e.severity_estimate).map_err(|e| StorageError::Clock(e.to_string()))?;
        let diff_summary = e.diff_summary.to_string();
        let propagated_to =
            serde_json::to_string(&e.propagated_to).map_err(|e| StorageError::Clock(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO change_events (
                change_event_id, change_type, target_resource_id, target_resource_type,
                changed_at, changed_by, source, description, diff_summary,
                related_commit, related_pr, severity_estimate, propagated_to,
                commit_sha, pipeline_url, git_repo, cluster_id, yaml_diff
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
            )
            ON CONFLICT(change_event_id) DO UPDATE SET
                change_type = excluded.change_type,
                target_resource_id = excluded.target_resource_id,
                target_resource_type = excluded.target_resource_type,
                changed_at = excluded.changed_at,
                changed_by = excluded.changed_by,
                source = excluded.source,
                description = excluded.description,
                diff_summary = excluded.diff_summary,
                related_commit = excluded.related_commit,
                related_pr = excluded.related_pr,
                severity_estimate = excluded.severity_estimate,
                propagated_to = excluded.propagated_to,
                commit_sha = excluded.commit_sha,
                pipeline_url = excluded.pipeline_url,
                git_repo = excluded.git_repo,
                cluster_id = excluded.cluster_id,
                yaml_diff = excluded.yaml_diff
            "#,
        )
        .bind(&e.change_event_id)
        .bind(&change_type)
        .bind(&e.target_resource_id)
        .bind(&e.target_resource_type)
        .bind(&e.changed_at)
        .bind(&e.changed_by)
        .bind(&source)
        .bind(&e.description)
        .bind(&diff_summary)
        .bind(&e.related_commit)
        .bind(&e.related_pr)
        .bind(&severity)
        .bind(&propagated_to)
        .bind(&e.commit_sha)
        .bind(&e.pipeline_url)
        .bind(&e.git_repo)
        .bind(&e.cluster_id)
        .bind(&e.yaml_diff)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 取单个 [`ChangeEvent`];不存在返 None。
    pub async fn get_change_event(&self, change_event_id: &str) -> Result<Option<ChangeEvent>, StorageError> {
        let row = sqlx::query(r#"SELECT * FROM change_events WHERE change_event_id = ?1"#)
            .bind(change_event_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_change_event).transpose()
    }

    /// 列 [`ChangeEvent`](新到旧,按 changed_at 降序)。
    pub async fn list_change_events(&self, limit: i64) -> Result<Vec<ChangeEvent>, StorageError> {
        let rows = sqlx::query(r#"SELECT * FROM change_events ORDER BY changed_at DESC LIMIT ?1"#)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_to_change_event).collect()
    }

    // ===== Phase 3.6: recovery_chains =====

    /// Insert or update a [`RecoveryChain`]。按 `chain_id` 幂等。
    pub async fn upsert_recovery_chain(&self, c: &RecoveryChain) -> Result<(), StorageError> {
        let status = serde_json::to_string(&c.status).map_err(|e| StorageError::Clock(e.to_string()))?;
        let on_failure = serde_json::to_string(&c.on_failure).map_err(|e| StorageError::Clock(e.to_string()))?;
        let step_executions =
            serde_json::to_string(&c.step_executions).map_err(|e| StorageError::Clock(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO recovery_chains (
                chain_id, template_id, target_resource_id, status, on_failure,
                step_executions, current_step_index, total_steps,
                initiated_by, request_reason, initiated_at, completed_at,
                approval_id, failure_reason, template_name, approval_comment, approved_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
            )
            ON CONFLICT(chain_id) DO UPDATE SET
                template_id = excluded.template_id,
                target_resource_id = excluded.target_resource_id,
                status = excluded.status,
                on_failure = excluded.on_failure,
                step_executions = excluded.step_executions,
                current_step_index = excluded.current_step_index,
                total_steps = excluded.total_steps,
                initiated_by = excluded.initiated_by,
                request_reason = excluded.request_reason,
                initiated_at = excluded.initiated_at,
                completed_at = excluded.completed_at,
                approval_id = excluded.approval_id,
                failure_reason = excluded.failure_reason,
                template_name = excluded.template_name,
                approval_comment = excluded.approval_comment,
                approved_at = excluded.approved_at
            "#,
        )
        .bind(&c.chain_id)
        .bind(&c.template_id)
        .bind(&c.target_resource_id)
        .bind(&status)
        .bind(&on_failure)
        .bind(&step_executions)
        .bind(c.current_step_index as i64)
        .bind(c.total_steps as i64)
        .bind(&c.initiated_by)
        .bind(&c.request_reason)
        .bind(&c.initiated_at)
        .bind(&c.completed_at)
        .bind(&c.approval_id)
        .bind(&c.failure_reason)
        .bind(&c.template_name)
        .bind(&c.approval_comment)
        .bind(&c.approved_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 取单个 [`RecoveryChain`];不存在返 None。
    pub async fn get_recovery_chain(&self, chain_id: &str) -> Result<Option<RecoveryChain>, StorageError> {
        let row = sqlx::query(r#"SELECT * FROM recovery_chains WHERE chain_id = ?1"#)
            .bind(chain_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_recovery_chain).transpose()
    }

    /// 列 [`RecoveryChain`](新到旧,按 initiated_at 降序)。
    pub async fn list_recovery_chains(&self, limit: i64) -> Result<Vec<RecoveryChain>, StorageError> {
        let rows = sqlx::query(r#"SELECT * FROM recovery_chains ORDER BY initiated_at DESC LIMIT ?1"#)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_to_recovery_chain).collect()
    }

    // ===== Phase 3.6: alert_events =====

    /// Insert or update an [`AlertEvent`]。按 `alert_event_id` 幂等。
    pub async fn upsert_alert_event(&self, a: &AlertEvent) -> Result<(), StorageError> {
        let severity = serde_json::to_string(&a.severity).map_err(|e| StorageError::Clock(e.to_string()))?;
        let status = serde_json::to_string(&a.status).map_err(|e| StorageError::Clock(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO alert_events (
                alert_event_id, alert_name, severity, status, fired_at,
                resource_ref, rule_id, metric_name, metric_value,
                summary, description, cluster_id, resolved_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
            )
            ON CONFLICT(alert_event_id) DO UPDATE SET
                alert_name = excluded.alert_name,
                severity = excluded.severity,
                status = excluded.status,
                fired_at = excluded.fired_at,
                resource_ref = excluded.resource_ref,
                rule_id = excluded.rule_id,
                metric_name = excluded.metric_name,
                metric_value = excluded.metric_value,
                summary = excluded.summary,
                description = excluded.description,
                cluster_id = excluded.cluster_id,
                resolved_at = excluded.resolved_at
            "#,
        )
        .bind(&a.alert_event_id)
        .bind(&a.alert_name)
        .bind(&severity)
        .bind(&status)
        .bind(&a.fired_at)
        .bind(&a.resource_ref)
        .bind(&a.rule_id)
        .bind(&a.metric_name)
        .bind(a.metric_value)
        .bind(&a.summary)
        .bind(&a.description)
        .bind(&a.cluster_id)
        .bind(&a.resolved_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 取单个 [`AlertEvent`];不存在返 None。
    pub async fn get_alert_event(&self, alert_event_id: &str) -> Result<Option<AlertEvent>, StorageError> {
        let row = sqlx::query(r#"SELECT * FROM alert_events WHERE alert_event_id = ?1"#)
            .bind(alert_event_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_alert_event).transpose()
    }

    /// 列 [`AlertEvent`](新到旧,按 fired_at 降序)。
    pub async fn list_alert_events(&self, limit: i64) -> Result<Vec<AlertEvent>, StorageError> {
        let rows = sqlx::query(r#"SELECT * FROM alert_events ORDER BY fired_at DESC LIMIT ?1"#)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_to_alert_event).collect()
    }

    // ===== Phase 4.3: report_subscriptions =====

    /// Insert or update a [`ReportSubscription`]。按 `subscription_id` 幂等。
    pub async fn upsert_subscription(
        &self,
        s: &ReportSubscription,
    ) -> Result<(), StorageError> {
        let template_id = serde_json::to_string(&s.template_id)
            .map_err(|e| StorageError::Clock(e.to_string()))?;
        let scope = serde_json::to_string(&s.scope).map_err(|e| StorageError::Clock(e.to_string()))?;
        let modules =
            serde_json::to_string(&s.modules).map_err(|e| StorageError::Clock(e.to_string()))?;
        let recipients = serde_json::to_string(&s.recipients)
            .map_err(|e| StorageError::Clock(e.to_string()))?;
        let last_status = serde_json::to_string(&s.last_status)
            .map_err(|e| StorageError::Clock(e.to_string()))?;
        sqlx::query(
            r#"
            INSERT INTO report_subscriptions (
                subscription_id, template_id, scope, modules, cron, recipients,
                enabled, created_at, last_run_at, last_status, last_error, last_report_id
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12
            )
            ON CONFLICT(subscription_id) DO UPDATE SET
                template_id = excluded.template_id,
                scope = excluded.scope,
                modules = excluded.modules,
                cron = excluded.cron,
                recipients = excluded.recipients,
                enabled = excluded.enabled,
                created_at = excluded.created_at,
                last_run_at = excluded.last_run_at,
                last_status = excluded.last_status,
                last_error = excluded.last_error,
                last_report_id = excluded.last_report_id
            "#,
        )
        .bind(&s.subscription_id)
        .bind(&template_id)
        .bind(&scope)
        .bind(&modules)
        .bind(&s.cron)
        .bind(&recipients)
        .bind(s.enabled as i64)
        .bind(&s.created_at)
        .bind(&s.last_run_at)
        .bind(&last_status)
        .bind(&s.last_error)
        .bind(&s.last_report_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 取单个 [`ReportSubscription`];不存在返 None。
    pub async fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<ReportSubscription>, StorageError> {
        let row = sqlx::query(r#"SELECT * FROM report_subscriptions WHERE subscription_id = ?1"#)
            .bind(subscription_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_subscription).transpose()
    }

    /// 列 [`ReportSubscription`](新到旧,按 created_at 降序)。
    pub async fn list_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<ReportSubscription>, StorageError> {
        let rows =
            sqlx::query(r#"SELECT * FROM report_subscriptions ORDER BY created_at DESC LIMIT ?1"#)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter().map(row_to_subscription).collect()
    }

    /// 删除 [`ReportSubscription`];存在返 true。
    pub async fn delete_subscription(&self, subscription_id: &str) -> Result<bool, StorageError> {
        let res = sqlx::query(r#"DELETE FROM report_subscriptions WHERE subscription_id = ?1"#)
            .bind(subscription_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
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

/// 把一行 change_events 解析成 [`ChangeEvent`]。
fn row_to_change_event(row: sqlx::sqlite::SqliteRow) -> Result<ChangeEvent, StorageError> {
    let change_type: String = row.try_get("change_type")?;
    let source: String = row.try_get("source")?;
    let severity: String = row.try_get("severity_estimate")?;
    let diff_summary: String = row.try_get("diff_summary")?;
    let propagated_to: String = row.try_get("propagated_to")?;
    Ok(ChangeEvent {
        change_event_id: row.try_get("change_event_id")?,
        change_type: serde_json::from_str(&change_type)
            .map_err(|e| StorageError::Clock(format!("change_type parse: {e}")))?,
        target_resource_id: row.try_get("target_resource_id")?,
        target_resource_type: row.try_get("target_resource_type")?,
        changed_at: row.try_get("changed_at")?,
        changed_by: row.try_get("changed_by")?,
        source: serde_json::from_str(&source).map_err(|e| StorageError::Clock(format!("source parse: {e}")))?,
        description: row.try_get("description")?,
        diff_summary: serde_json::from_str(&diff_summary)
            .map_err(|e| StorageError::Clock(format!("diff_summary parse: {e}")))?,
        related_commit: row.try_get("related_commit")?,
        related_pr: row.try_get("related_pr")?,
        severity_estimate: serde_json::from_str(&severity)
            .map_err(|e| StorageError::Clock(format!("severity parse: {e}")))?,
        propagated_to: serde_json::from_str(&propagated_to)
            .map_err(|e| StorageError::Clock(format!("propagated_to parse: {e}")))?,
        commit_sha: row.try_get("commit_sha")?,
        pipeline_url: row.try_get("pipeline_url")?,
        git_repo: row.try_get("git_repo")?,
        cluster_id: row.try_get("cluster_id")?,
        yaml_diff: row.try_get("yaml_diff")?,
    })
}

/// 把一行 recovery_chains 解析成 [`RecoveryChain`]。
fn row_to_recovery_chain(row: sqlx::sqlite::SqliteRow) -> Result<RecoveryChain, StorageError> {
    let status: String = row.try_get("status")?;
    let on_failure: String = row.try_get("on_failure")?;
    let step_executions: String = row.try_get("step_executions")?;
    let current_step_index: i64 = row.try_get("current_step_index")?;
    let total_steps: i64 = row.try_get("total_steps")?;
    Ok(RecoveryChain {
        chain_id: row.try_get("chain_id")?,
        template_id: row.try_get("template_id")?,
        target_resource_id: row.try_get("target_resource_id")?,
        status: serde_json::from_str(&status).map_err(|e| StorageError::Clock(format!("status parse: {e}")))?,
        on_failure: serde_json::from_str(&on_failure)
            .map_err(|e| StorageError::Clock(format!("on_failure parse: {e}")))?,
        step_executions: serde_json::from_str(&step_executions)
            .map_err(|e| StorageError::Clock(format!("step_executions parse: {e}")))?,
        current_step_index: current_step_index as usize,
        total_steps: total_steps as usize,
        initiated_by: row.try_get("initiated_by")?,
        request_reason: row.try_get("request_reason")?,
        initiated_at: row.try_get("initiated_at")?,
        completed_at: row.try_get("completed_at")?,
        approval_id: row.try_get("approval_id")?,
        failure_reason: row.try_get("failure_reason")?,
        template_name: row.try_get("template_name")?,
        approval_comment: row.try_get("approval_comment")?,
        approved_at: row.try_get("approved_at")?,
    })
}

/// 把一行 alert_events 解析成 [`AlertEvent`]。
fn row_to_alert_event(row: sqlx::sqlite::SqliteRow) -> Result<AlertEvent, StorageError> {
    let severity: String = row.try_get("severity")?;
    let status: String = row.try_get("status")?;
    Ok(AlertEvent {
        alert_event_id: row.try_get("alert_event_id")?,
        alert_name: row.try_get("alert_name")?,
        severity: serde_json::from_str(&severity).map_err(|e| StorageError::Clock(format!("severity parse: {e}")))?,
        status: serde_json::from_str(&status).map_err(|e| StorageError::Clock(format!("status parse: {e}")))?,
        fired_at: row.try_get("fired_at")?,
        resource_ref: row.try_get("resource_ref")?,
        rule_id: row.try_get("rule_id")?,
        metric_name: row.try_get("metric_name")?,
        metric_value: row.try_get("metric_value")?,
        summary: row.try_get("summary")?,
        description: row.try_get("description")?,
        cluster_id: row.try_get("cluster_id")?,
        resolved_at: row.try_get("resolved_at")?,
    })
}

/// 把一行 report_subscriptions 解析成 [`ReportSubscription`]。
fn row_to_subscription(row: sqlx::sqlite::SqliteRow) -> Result<ReportSubscription, StorageError> {
    let template_id: String = row.try_get("template_id")?;
    let scope: String = row.try_get("scope")?;
    let modules: String = row.try_get("modules")?;
    let recipients: String = row.try_get("recipients")?;
    let last_status: String = row.try_get("last_status")?;
    let enabled: i64 = row.try_get("enabled")?;
    Ok(ReportSubscription {
        subscription_id: row.try_get("subscription_id")?,
        template_id: serde_json::from_str(&template_id)
            .map_err(|e| StorageError::Clock(format!("template_id parse: {e}")))?,
        scope: serde_json::from_str(&scope).map_err(|e| StorageError::Clock(format!("scope parse: {e}")))?,
        modules: serde_json::from_str(&modules).map_err(|e| StorageError::Clock(format!("modules parse: {e}")))?,
        cron: row.try_get("cron")?,
        recipients: serde_json::from_str(&recipients)
            .map_err(|e| StorageError::Clock(format!("recipients parse: {e}")))?,
        enabled: enabled != 0,
        created_at: row.try_get("created_at")?,
        last_run_at: row.try_get("last_run_at")?,
        last_status: serde_json::from_str(&last_status)
            .map_err(|e| StorageError::Clock(format!("last_status parse: {e}")))?,
        last_error: row.try_get("last_error")?,
        last_report_id: row.try_get("last_report_id")?,
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
    async fn latest_topology_facts_includes_edge_facts() {
        // Phase 3.7:topology-edge fact 也应被 latest_topology_facts 取出(与 node 一起)
        let store = migrated_store().await;
        store
            .upsert_facts(&[
                fact("pod-a", "pod:demo:default:web-0", "Pod", 10, "{}"),
                fact("node-b", "node:demo:worker-1", "Node", 10, "{}"),
                Fact::new(
                    "edge-1",
                    "topology-edge",
                    "k8s",
                    "edge:SCHEDULED_ON:pod:demo:default:web-0->node:demo:worker-1",
                    "Edge",
                    10,
                    r#"{"source":"pod:demo:default:web-0","target":"node:demo:worker-1","edge_type":"SCHEDULED_ON"}"#,
                ),
            ])
            .await
            .expect("upsert facts");

        let latest = store
            .latest_topology_facts()
            .await
            .expect("query latest topology facts");
        // 2 node + 1 edge
        assert_eq!(latest.len(), 3);
        let kinds: Vec<&str> = latest.iter().map(|f| f.kind.as_str()).collect();
        assert!(kinds.contains(&"topology-node"));
        assert!(kinds.contains(&"topology-edge"));
        let edge = latest
            .iter()
            .find(|f| f.kind == "topology-edge")
            .expect("edge fact present");
        assert_eq!(
            edge.resource_id,
            "edge:SCHEDULED_ON:pod:demo:default:web-0->node:demo:worker-1"
        );
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
            &engine_recovery::MockHandlerExecutor,
        )
        .await
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

    #[tokio::test]
    async fn change_event_round_trips_sqlite() {
        let store = migrated_store().await;
        let ev = engine_changes::ChangeEvent {
            change_event_id: "ce-abc123def456".into(),
            change_type: engine_changes::ChangeType::ConfigmapUpdated,
            target_resource_id: "cm:order-config".into(),
            target_resource_type: "ConfigMap".into(),
            changed_at: "2026-07-11T03:00:00Z".into(),
            changed_by: "alice".into(),
            source: engine_changes::Source::Manual,
            description: "pool 20->50".into(),
            diff_summary: serde_json::json!({"max_pool_size": {"old": 20, "new": 50}}),
            related_commit: "abc123".into(),
            related_pr: "PR-42".into(),
            severity_estimate: engine_changes::Severity::Medium,
            propagated_to: vec!["pod:order-api-1".into(), "pod:order-api-2".into()],
            commit_sha: "abc123".into(),
            pipeline_url: "https://ci/x".into(),
            git_repo: "order-api".into(),
            cluster_id: "demo".into(),
            yaml_diff: "--- old\n+++ new\n".into(),
        };
        store.upsert_change_event(&ev).await.expect("upsert");
        let got = store
            .get_change_event(&ev.change_event_id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(got, ev);
        assert_eq!(got.propagated_to, vec!["pod:order-api-1".to_string(), "pod:order-api-2".to_string()]);
        assert_eq!(got.diff_summary["max_pool_size"]["new"], 50);

        let listed = store.list_change_events(10).await.expect("list");
        assert_eq!(listed.len(), 1);
        // upsert 幂等
        store.upsert_change_event(&ev).await.expect("upsert again");
        let listed2 = store.list_change_events(10).await.expect("list2");
        assert_eq!(listed2.len(), 1);
    }

    #[tokio::test]
    async fn recovery_chain_round_trips_sqlite() {
        let store = migrated_store().await;
        let chain = engine_recovery::RecoveryChain {
            chain_id: "chain-1".into(),
            template_id: "safe_rollback_deployment".into(),
            target_resource_id: "deploy:order-api".into(),
            status: engine_recovery::ChainStatus::Succeeded,
            on_failure: engine_recovery::OnFailureStrategy::RollbackAll,
            step_executions: vec!["e1".into(), "e2".into(), "e3".into()],
            current_step_index: 3,
            total_steps: 3,
            initiated_by: "tester".into(),
            request_reason: "rollback".into(),
            initiated_at: "2026-07-11T03:00:00Z".into(),
            completed_at: "2026-07-11T03:01:00Z".into(),
            approval_id: String::new(),
            failure_reason: String::new(),
            template_name: "安全回滚 Deployment".into(),
            approval_comment: String::new(),
            approved_at: String::new(),
        };
        store.upsert_recovery_chain(&chain).await.expect("upsert");
        let got = store
            .get_recovery_chain(&chain.chain_id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(got.chain_id, "chain-1");
        assert_eq!(got.status, engine_recovery::ChainStatus::Succeeded);
        assert_eq!(got.on_failure, engine_recovery::OnFailureStrategy::RollbackAll);
        assert_eq!(got.step_executions, vec!["e1".to_string(), "e2".to_string(), "e3".to_string()]);
        assert_eq!(got.current_step_index, 3);
        assert_eq!(got.total_steps, 3);
        assert_eq!(got.template_name, "安全回滚 Deployment");

        let listed = store.list_recovery_chains(10).await.expect("list");
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn alert_event_round_trips_sqlite() {
        let store = migrated_store().await;
        let mut alert = engine_changes::AlertEvent::new("alert-1", "HighErrorRate");
        alert.severity = engine_changes::AlertSeverity::Critical;
        alert.status = engine_changes::AlertStatus::Firing;
        alert.fired_at = "2026-07-11T03:00:00Z".into();
        alert.resource_ref = "svc:order-api".into();
        alert.rule_id = "rule-1".into();
        alert.metric_name = "error_rate".into();
        alert.metric_value = 12.5;
        alert.summary = "error rate high".into();
        alert.description = "p99 spike".into();
        alert.cluster_id = "demo".into();
        store.upsert_alert_event(&alert).await.expect("upsert");
        let got = store
            .get_alert_event(&alert.alert_event_id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(got.alert_event_id, "alert-1");
        assert_eq!(got.severity, engine_changes::AlertSeverity::Critical);
        assert_eq!(got.status, engine_changes::AlertStatus::Firing);
        assert_eq!(got.resource_ref, "svc:order-api");
        assert!((got.metric_value - 12.5).abs() < f64::EPSILON);

        let listed = store.list_alert_events(10).await.expect("list");
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn subscription_round_trips_sqlite() {
        let store = migrated_store().await;
        let sub = engine_reports::ReportSubscription {
            subscription_id: "sub-test1".into(),
            template_id: engine_reports::ReportTemplate::ApplicationHealth,
            scope: engine_reports::ReportScope {
                application_id: Some("app:order".into()),
                cluster_id: None,
                change_event_id: None,
                fault_id: None,
                time_range_start: None,
                time_range_end: None,
            },
            modules: vec!["health_score".into(), "risk_list".into()],
            cron: "0 9 * * 1".into(),
            recipients: vec!["ops@example.com".into(), "sre@example.com".into()],
            enabled: true,
            created_at: "2026-07-20T00:00:00Z".into(),
            last_run_at: "2026-07-20T09:00:00Z".into(),
            last_status: engine_reports::SubscriptionStatus::Ok,
            last_error: String::new(),
            last_report_id: "rpt-abc".into(),
        };
        store.upsert_subscription(&sub).await.expect("upsert");

        let got = store
            .get_subscription(&sub.subscription_id)
            .await
            .expect("get")
            .expect("present");
        assert_eq!(got.subscription_id, "sub-test1");
        assert_eq!(got.template_id, engine_reports::ReportTemplate::ApplicationHealth);
        assert_eq!(got.scope.application_id.as_deref(), Some("app:order"));
        assert_eq!(got.modules, vec!["health_score".to_string(), "risk_list".to_string()]);
        assert_eq!(got.cron, "0 9 * * 1");
        assert_eq!(got.recipients, vec!["ops@example.com".to_string(), "sre@example.com".to_string()]);
        assert!(got.enabled);
        assert_eq!(got.last_status, engine_reports::SubscriptionStatus::Ok);
        assert_eq!(got.last_report_id, "rpt-abc");

        // upsert 幂等(更新 last_status)
        let mut updated = got;
        updated.last_status = engine_reports::SubscriptionStatus::Failed;
        updated.last_error = "boom".into();
        store.upsert_subscription(&updated).await.expect("upsert2");
        let got2 = store.get_subscription("sub-test1").await.expect("get").expect("present");
        assert_eq!(got2.last_status, engine_reports::SubscriptionStatus::Failed);
        assert_eq!(got2.last_error, "boom");

        // list
        let listed = store.list_subscriptions(10).await.expect("list");
        assert_eq!(listed.len(), 1);

        // delete
        assert!(store.delete_subscription("sub-test1").await.expect("del"));
        assert!(store.get_subscription("sub-test1").await.expect("get").is_none());
        assert!(!store.delete_subscription("sub-test1").await.expect("del2"));
    }
}
