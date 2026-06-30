//! SQLite storage backend.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use engine_core::Fact;
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
}
