use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

use crate::{
    models::{AppSettings, ProviderSnapshot},
    providers::CacheIdentity,
};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("OpenQuota data directory could not be created")]
    CreateDirectory(#[source] std::io::Error),
    #[error("OpenQuota database is unavailable")]
    Database(#[from] rusqlite::Error),
    #[error("Cached OpenQuota data is invalid")]
    InvalidCache(#[from] serde_json::Error),
    #[error("OpenQuota database lock is unavailable")]
    Poisoned,
}

pub struct Storage {
    connection: Mutex<Connection>,
}

pub struct ProviderAccountUpdate {
    pub provider_family: String,
    pub identity_key: String,
    pub provider_id: String,
    pub payload: String,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(StorageError::CreateDirectory)?;
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             DROP TABLE IF EXISTS daily_usage;
             DROP TABLE IF EXISTS log_file_cache;
             CREATE TABLE IF NOT EXISTS provider_snapshots (
               provider_id TEXT PRIMARY KEY,
               payload TEXT NOT NULL,
               refreshed_at TEXT NOT NULL,
               identity_key TEXT
             );
             CREATE TABLE IF NOT EXISTS app_settings (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS panel_state (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               height INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS provider_environment (
               id INTEGER PRIMARY KEY CHECK (id = 1),
               payload TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS provider_account_records (
               provider_family TEXT NOT NULL,
               identity_key TEXT NOT NULL,
               provider_id TEXT NOT NULL,
               payload TEXT NOT NULL,
               PRIMARY KEY(provider_family, identity_key),
               UNIQUE(provider_family, provider_id)
             );",
        )?;
        if !Self::has_column(&connection, "provider_snapshots", "identity_key")? {
            connection.execute(
                "ALTER TABLE provider_snapshots ADD COLUMN identity_key TEXT",
                [],
            )?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[cfg(test)]
    pub fn load_snapshot(
        &self,
        provider_id: &str,
    ) -> Result<Option<ProviderSnapshot>, StorageError> {
        self.load_snapshot_for_identity(provider_id, CacheIdentity::Unscoped)
    }

    pub fn load_snapshot_for_identity(
        &self,
        provider_id: &str,
        identity: CacheIdentity<'_>,
    ) -> Result<Option<ProviderSnapshot>, StorageError> {
        let connection = self.connection()?;
        let cached: Option<(String, Option<String>)> = connection
            .query_row(
                "SELECT payload, identity_key FROM provider_snapshots WHERE provider_id = ?1",
                [provider_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        cached
            .filter(|(_, cached_identity)| match identity {
                CacheIdentity::Unscoped => cached_identity.is_none(),
                CacheIdentity::Resolved(expected) => cached_identity.as_deref() == Some(expected),
                CacheIdentity::Unresolved => true,
            })
            .map(|json| {
                let mut snapshot: ProviderSnapshot = serde_json::from_str(&json.0)?;
                if snapshot.account_identity.is_none() {
                    snapshot.account_identity =
                        crate::providers::identity::account_fact(provider_id, json.1.as_deref());
                }
                Ok(crate::providers::cache::cloud_facts(snapshot))
            })
            .transpose()
    }

    #[cfg(test)]
    pub fn save_snapshot(&self, snapshot: &ProviderSnapshot) -> Result<(), StorageError> {
        self.save_snapshot_for_identity(snapshot, None)
    }

    pub fn save_snapshot_for_identity(
        &self,
        snapshot: &ProviderSnapshot,
        identity_key: Option<&str>,
    ) -> Result<(), StorageError> {
        let payload = serde_json::to_string(snapshot)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO provider_snapshots(provider_id, payload, refreshed_at, identity_key)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_id) DO UPDATE SET
               payload = excluded.payload,
               refreshed_at = excluded.refreshed_at,
               identity_key = excluded.identity_key",
            params![
                snapshot.provider_id,
                payload,
                snapshot.refreshed_at.to_rfc3339(),
                identity_key,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn load_settings(&self) -> Result<Option<AppSettings>, StorageError> {
        let connection = self.connection()?;
        let payload: Option<String> = connection
            .query_row("SELECT payload FROM app_settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        payload
            .map(|json| serde_json::from_str(&json).map_err(StorageError::from))
            .transpose()
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), StorageError> {
        self.save_settings_with_account_updates(settings, &[])
    }

    pub fn save_settings_with_account_updates(
        &self,
        settings: &AppSettings,
        account_updates: &[ProviderAccountUpdate],
    ) -> Result<(), StorageError> {
        let payload = serde_json::to_string(settings)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        for update in account_updates {
            transaction.execute(
                "INSERT INTO provider_account_records(
                   provider_family, identity_key, provider_id, payload
                 ) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(provider_family, identity_key) DO UPDATE SET
                   provider_id = excluded.provider_id,
                   payload = excluded.payload",
                params![
                    update.provider_family,
                    update.identity_key,
                    update.provider_id,
                    update.payload,
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO app_settings(id, payload) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
            [payload],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn load_provider_environment(
        &self,
    ) -> Result<Option<HashMap<String, String>>, StorageError> {
        let connection = self.connection()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT payload FROM provider_environment WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(payload.and_then(|json| serde_json::from_str(&json).ok()))
    }

    #[cfg(unix)]
    pub fn save_provider_environment(
        &self,
        environment: &HashMap<String, String>,
    ) -> Result<(), StorageError> {
        let payload = serde_json::to_string(environment)?;
        self.connection()?.execute(
            "INSERT INTO provider_environment(id, payload) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
            [payload],
        )?;
        Ok(())
    }

    pub fn load_provider_account_records(
        &self,
        provider_family: &str,
    ) -> Result<Vec<(String, String, String)>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT identity_key, provider_id, payload
             FROM provider_account_records
             WHERE provider_family = ?1
             ORDER BY provider_id",
        )?;
        let records = statement
            .query_map([provider_family], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)?;
        Ok(records)
    }

    pub fn load_all_provider_account_records(
        &self,
    ) -> Result<Vec<(String, String, String, String)>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT provider_family, identity_key, provider_id, payload
             FROM provider_account_records
             ORDER BY provider_family, provider_id, identity_key",
        )?;
        let records = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)?;
        Ok(records)
    }

    pub fn load_observed_account_provider_ids(&self) -> Result<Vec<String>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT DISTINCT provider_id
             FROM provider_account_records
             ORDER BY provider_id",
        )?;
        let provider_ids = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)?;
        Ok(provider_ids)
    }

    pub fn save_provider_account_record(
        &self,
        provider_family: &str,
        identity_key: &str,
        provider_id: &str,
        payload: &str,
    ) -> Result<(), StorageError> {
        self.connection()?.execute(
            "INSERT INTO provider_account_records(
               provider_family, identity_key, provider_id, payload
             ) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(provider_family, identity_key) DO UPDATE SET
               provider_id = excluded.provider_id,
               payload = excluded.payload",
            params![provider_family, identity_key, provider_id, payload],
        )?;
        Ok(())
    }

    pub fn load_panel_height(&self) -> Result<Option<u32>, StorageError> {
        let connection = self.connection()?;
        let height = connection
            .query_row("SELECT height FROM panel_state WHERE id = 1", [], |row| {
                row.get::<_, i64>(0)
            })
            .optional()?;
        Ok(height.and_then(|value| u32::try_from(value).ok()))
    }

    pub fn save_panel_height(&self, height: u32) -> Result<(), StorageError> {
        self.connection()?.execute(
            "INSERT INTO panel_state(id, height) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET height = excluded.height",
            [i64::from(height)],
        )?;
        Ok(())
    }

    pub fn clear_panel_height(&self) -> Result<(), StorageError> {
        self.connection()?
            .execute("DELETE FROM panel_state WHERE id = 1", [])?;
        Ok(())
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.connection.lock().map_err(|_| StorageError::Poisoned)
    }

    fn has_column(
        connection: &Connection,
        table: &str,
        column: &str,
    ) -> Result<bool, rusqlite::Error> {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(columns.iter().any(|name| name == column))
    }
}

#[cfg(test)]
mod tests {

    use chrono::Utc;
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{ProviderAccountUpdate, Storage};
    use crate::models::{AppSettings, ProviderSnapshot, QuotaFormat, QuotaWindow};
    use crate::providers::CacheIdentity;

    #[test]
    fn snapshot_round_trip_contains_no_credentials() {
        let directory = tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("openquota.db")).unwrap();
        let snapshot = ProviderSnapshot {
            provider_id: "codex".into(),
            plan: Some("Plus".into()),
            quotas: Vec::new(),
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: Vec::new(),
            warnings: Vec::new(),
            refreshed_at: Utc::now(),
            remembered: false,
            account_identity: None,
            shared_scope: None,
        };

        storage.save_snapshot(&snapshot).unwrap();

        assert_eq!(storage.load_snapshot("codex").unwrap(), Some(snapshot));
        let bytes = std::fs::read(directory.path().join("openquota.db")).unwrap();
        let database = String::from_utf8_lossy(&bytes);
        assert!(!database.contains("access_token"));
        assert!(!database.contains("refresh_token"));
    }

    #[test]
    fn account_scoped_snapshot_is_visible_only_to_the_same_identity() {
        let directory = tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("openquota.db")).unwrap();
        let snapshot = ProviderSnapshot {
            provider_id: "claude".into(),
            plan: Some("Max".into()),
            quotas: Vec::new(),
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: Vec::new(),
            warnings: Vec::new(),
            refreshed_at: Utc::now(),
            remembered: false,
            account_identity: crate::providers::identity::account_fact(
                "claude",
                Some("identity-a"),
            ),
            shared_scope: None,
        };

        storage
            .save_snapshot_for_identity(&snapshot, Some("identity-a"))
            .unwrap();

        assert_eq!(
            storage
                .load_snapshot_for_identity("claude", CacheIdentity::Resolved("identity-a"))
                .unwrap(),
            Some(snapshot.clone())
        );
        assert!(storage
            .load_snapshot_for_identity("claude", CacheIdentity::Resolved("identity-b"))
            .unwrap()
            .is_none());
        assert!(storage.load_snapshot("claude").unwrap().is_none());
        assert_eq!(
            storage
                .load_snapshot_for_identity("claude", CacheIdentity::Unresolved)
                .unwrap(),
            Some(snapshot)
        );
    }

    #[test]
    fn provider_account_records_round_trip_by_family() {
        let directory = tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("openquota.db")).unwrap();

        storage
            .save_provider_account_record(
                "claude",
                "identity-a",
                "claude@12345678",
                r#"{"displayName":"Claude — Work"}"#,
            )
            .unwrap();

        assert_eq!(
            storage.load_provider_account_records("claude").unwrap(),
            [(
                "identity-a".to_owned(),
                "claude@12345678".to_owned(),
                r#"{"displayName":"Claude — Work"}"#.to_owned(),
            )]
        );
        assert!(storage
            .load_provider_account_records("codex")
            .unwrap()
            .is_empty());
        assert_eq!(
            storage.load_observed_account_provider_ids().unwrap(),
            ["claude@12345678"]
        );
    }

    #[test]
    fn account_records_and_settings_roll_back_together() {
        let directory = tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("openquota.db")).unwrap();
        let original = AppSettings {
            theme: crate::models::ThemePreference::Light,
            ..AppSettings::default()
        };
        storage.save_settings(&original).unwrap();
        storage
            .save_provider_account_record("codex", "identity-a", "codex", r#"{"name":"old"}"#)
            .unwrap();
        let changed = AppSettings {
            theme: crate::models::ThemePreference::Dark,
            ..original.clone()
        };
        let updates = [
            ProviderAccountUpdate {
                provider_family: "codex".into(),
                identity_key: "identity-a".into(),
                provider_id: "codex".into(),
                payload: r#"{"name":"changed"}"#.into(),
            },
            ProviderAccountUpdate {
                provider_family: "codex".into(),
                identity_key: "identity-b".into(),
                provider_id: "codex".into(),
                payload: "{}".into(),
            },
        ];

        assert!(storage
            .save_settings_with_account_updates(&changed, &updates)
            .is_err());
        assert_eq!(storage.load_settings().unwrap(), Some(original));
        let records = storage.load_provider_account_records("codex").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].2, r#"{"name":"old"}"#);
    }

    #[test]
    fn legacy_snapshot_table_gains_the_account_identity_column() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("openquota.db");
        let legacy = Connection::open(&path).unwrap();
        legacy
            .execute_batch(
                "CREATE TABLE provider_snapshots (
                   provider_id TEXT PRIMARY KEY,
                   payload TEXT NOT NULL,
                   refreshed_at TEXT NOT NULL
                 );",
            )
            .unwrap();
        drop(legacy);

        let storage = Storage::open(&path).unwrap();
        let connection = storage.connection().unwrap();

        assert!(Storage::has_column(&connection, "provider_snapshots", "identity_key").unwrap());
    }

    #[test]
    fn legacy_count_quota_does_not_invent_an_unknown_unit() {
        let directory = tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("openquota.db")).unwrap();
        let snapshot = ProviderSnapshot {
            provider_id: "cursor".into(),
            plan: None,
            quotas: vec![QuotaWindow {
                id: "requests".into(),
                label: "Requests".into(),
                used_percent: 25.0,
                resets_at: None,
                period_seconds: 2_592_000,
                format: QuotaFormat::Count,
                used_value: Some(25.0),
                limit_value: Some(100.0),
                remaining_value: None,
                unit: None,
                estimated: false,
                source_note: None,
            }],
            value_metrics: Vec::new(),
            status_metrics: Vec::new(),
            notices: Vec::new(),
            warnings: Vec::new(),
            refreshed_at: Utc::now(),
            remembered: false,
            account_identity: None,
            shared_scope: None,
        };

        storage.save_snapshot(&snapshot).unwrap();

        assert_eq!(
            storage.load_snapshot("cursor").unwrap().unwrap().quotas[0].unit,
            None
        );
    }

    #[test]
    fn settings_round_trip_uses_the_same_disk_database() {
        let directory = tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("openquota.db")).unwrap();
        let settings = AppSettings {
            always_show_pacing: true,
            ..AppSettings::default()
        };
        storage.save_settings(&settings).unwrap();
        assert_eq!(storage.load_settings().unwrap(), Some(settings));
    }

    #[test]
    fn panel_height_round_trip_is_independent_from_app_settings() {
        let directory = tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("openquota.db")).unwrap();
        let settings = AppSettings::default();
        storage.save_settings(&settings).unwrap();

        assert_eq!(storage.load_panel_height().unwrap(), None);
        storage.save_panel_height(734).unwrap();

        assert_eq!(storage.load_panel_height().unwrap(), Some(734));
        assert_eq!(storage.load_settings().unwrap(), Some(settings));

        storage.clear_panel_height().unwrap();
        assert_eq!(storage.load_panel_height().unwrap(), None);
    }
    #[test]
    fn obsolete_tables_and_payloads_do_not_break_existing_databases() {
        for timestamp in ["modified_millis", "modified_nanos"] {
            let directory = tempdir().unwrap();
            let path = directory.path().join("openquota.db");
            let legacy = Connection::open(&path).unwrap();
            legacy.execute_batch(&format!("CREATE TABLE daily_usage (tokens INTEGER); INSERT INTO daily_usage VALUES (42); CREATE TABLE log_file_cache ({timestamp} INTEGER); INSERT INTO log_file_cache VALUES (1); CREATE TABLE app_settings (id INTEGER PRIMARY KEY, payload TEXT NOT NULL); INSERT INTO app_settings VALUES (1, '{{\"theme\":\"dark\",\"showTotalSpend\":true,\"totalSpendPeriod\":\"today\"}}');")).unwrap();
            drop(legacy);
            for _ in 0..2 {
                let storage = Storage::open(&path).unwrap();
                let count: i64 = storage.connection().unwrap().query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('daily_usage', 'log_file_cache')", [], |row| row.get(0)).unwrap();
                assert_eq!(count, 0);
                assert_eq!(
                    storage.load_settings().unwrap().unwrap().theme,
                    crate::models::ThemePreference::Dark
                );
                let snapshot: ProviderSnapshot = serde_json::from_value(serde_json::json!({
                    "providerId":"claude", "plan":null, "quotas":[], "warnings":[], "refreshedAt":"2026-09-26T00:00:00Z",
                    "usage":{"today":{"tokens":42}, "unknownModels":["old"]}
                })).unwrap();
                storage.save_snapshot(&snapshot).unwrap();
                assert_eq!(
                    storage.load_snapshot("claude").unwrap(),
                    Some(snapshot.clone())
                );
                assert!(serde_json::to_value(snapshot)
                    .unwrap()
                    .get("usage")
                    .is_none());
            }
        }
    }
}
