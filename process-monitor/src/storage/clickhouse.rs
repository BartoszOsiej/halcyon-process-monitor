// ── storage/clickhouse.rs — ClickHouse event storage ─────────────────────
//
// Ingests detection events into ClickHouse for:
//   - Long-term retention and audit trail
//   - Analytical queries (top processes, extension frequency, alert history)
//   - Dashboard materialized views
//
// Uses batch inserts with configurable:
//   - Connection URL (default: http://localhost:8123)
//   - Database name (default: halcyon)
//   - Batch size and flush interval
//
// Schema (auto-created on startup):
//   CREATE TABLE IF NOT EXISTS halcyon.events (
//       ts DateTime64(3),
//       kind LowCardinality(String),
//       pid UInt32,
//       uid UInt32,
//       comm LowCardinality(String),
//       file Nullable(String),
//       extension LowCardinality(Nullable(String)),
//       argv Nullable(String),
//       event_time DateTime DEFAULT now()
//   ) ENGINE = MergeTree()
//   PARTITION BY toYYYYMMDD(ts)
//   ORDER BY (ts, kind, pid);

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;

use super::StorageEvent;

/// ClickHouse storage configuration.
pub struct ClickHouseConfig {
    pub url: String,
    pub database: String,
    pub batch_size: usize,
    pub flush_interval_ms: u64,
}

impl Default for ClickHouseConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8123".into(),
            database: "halcyon".into(),
            batch_size: 1000,
            flush_interval_ms: 1000,
        }
    }
}

/// ClickHouse event store. Buffers events and flushes in batches.
pub struct ClickHouseStore {
    buffer: Arc<Mutex<Vec<StorageEvent>>>,
    client: Client,
    config: ClickHouseConfig,
}

impl ClickHouseStore {
    /// Create a new ClickHouse store. Initializes the schema.
    pub fn start(config: ClickHouseConfig) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("failed to create HTTP client: {e}"))?;

        let store = Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            client,
            config: config.clone(),
        };

        // Create database and table
        store.init_schema()?;

        // Start flush thread
        let buffer = store.buffer.clone();
        let client = store.client.clone();
        let cfg = store.config.clone();

        thread::Builder::new()
            .name("clickhouse-flush".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_millis(cfg.flush_interval_ms));
                    let batch: Vec<StorageEvent> = {
                        let mut buf = buffer.lock().unwrap();
                        if buf.is_empty() {
                            continue;
                        }
                        let batch: Vec<StorageEvent> = buf.drain(..).collect();
                        batch
                    };

                    if let Err(e) = flush_batch(&client, &cfg.url, &cfg.database, &batch) {
                        eprintln!("[clickhouse] flush failed: {e}");
                    }
                }
            })
            .map_err(|e| format!("failed to spawn flush thread: {e}"))?;

        eprintln!("[clickhouse] connected to {} db={}", config.url, config.database);
        Ok(store)
    }

    /// Create database and table if they don't exist.
    fn init_schema(&self) -> Result<(), String> {
        let create_db = format!("CREATE DATABASE IF NOT EXISTS {}", self.config.database);
        self.execute(&create_db)?;

        let create_table = format!(
            "CREATE TABLE IF NOT EXISTS {}.events (
                ts DateTime64(3),
                kind LowCardinality(String),
                pid UInt32,
                uid UInt32,
                comm LowCardinality(String),
                file Nullable(String),
                extension LowCardinality(Nullable(String)),
                argv Nullable(String),
                event_time DateTime DEFAULT now()
            ) ENGINE = MergeTree()
            PARTITION BY toYYYYMMDD(ts)
            ORDER BY (ts, kind, pid)",
            self.config.database
        );
        self.execute(&create_table)?;

        eprintln!("[clickhouse] schema initialized");
        Ok(())
    }

    /// Execute a DDL/DML query.
    fn execute(&self, query: &str) -> Result<(), String> {
        let url = format!("{}/?query={}", self.config.url, urlencoding::encode(query));
        self.client
            .post(&url)
            .body("")
            .send()
            .map_err(|e| format!("clickhouse query failed: {e}"))?;
        Ok(())
    }

    /// Add an event to the buffer. Will be flushed in the next batch.
    pub fn insert(&self, event: StorageEvent) {
        let mut buf = self.buffer.lock().unwrap();
        buf.push(event);
        if buf.len() >= self.config.batch_size {
            // Force immediate flush if buffer is full
            let batch: Vec<StorageEvent> = buf.drain(..).collect();
            drop(buf); // Release lock before IO
            let _ = flush_batch(&self.client, &self.config.url, &self.config.database, &batch);
        }
    }
}

/// Flush a batch of events to ClickHouse using INSERT format.
fn flush_batch(
    client: &Client,
    url: &str,
    database: &str,
    events: &[StorageEvent],
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }

    // Build TSV INSERT data (ClickHouse native format)
    let mut values = String::with_capacity(events.len() * 200);
    for ev in events {
        let file = ev.file.as_deref().unwrap_or("\\N");
        let ext = ev.extension.as_deref().unwrap_or("\\N");
        let argv = ev.argv.as_deref().unwrap_or("\\N");
        values.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            ev.ts, ev.kind, ev.pid, ev.uid, ev.comm, file, ext, argv
        ));
    }

    let query = format!(
        "INSERT INTO {}.events (ts, kind, pid, uid, comm, file, extension, argv) FORMAT TabSeparated",
        database
    );

    let url = format!("{}/?query={}", url, urlencoding::encode(&query));
    let resp = client
        .post(&url)
        .body(values)
        .send()
        .map_err(|e| format!("clickhouse insert failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("clickhouse insert error {status}: {body}"));
    }

    Ok(())
}

/// Query helper — returns top-N processes by event count.
pub fn query_top_processes(
    client: &Client,
    url: &str,
    database: &str,
    n: usize,
) -> Result<Vec<(String, u64)>, String> {
    let query = format!(
        "SELECT comm, count() as cnt FROM {}.events WHERE kind = 'Open' GROUP BY comm ORDER BY cnt DESC LIMIT {}",
        database, n
    );
    let url = format!("{}/?query={}&default_format=TabSeparatedWithNames", url, urlencoding::encode(&query));
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("clickhouse query failed: {e}"))?;
    let body = resp.text().map_err(|e| format!("read failed: {e}"))?;

    let mut results = Vec::new();
    for line in body.lines().skip(1) {
        // Skip header
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 2 {
            if let Ok(count) = parts[1].parse::<u64>() {
                results.push((parts[0].to_string(), count));
            }
        }
    }
    Ok(results)
}
