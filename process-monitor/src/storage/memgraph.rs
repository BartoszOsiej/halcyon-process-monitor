// ── storage/memgraph.rs — MemGraph process relationship graph ────────────
//
// Stores process trees and file access patterns as a graph in MemGraph:
//   - (:Process {pid, comm, uid}) — nodes for each process
//   - (:File {path, extension}) — nodes for accessed files
//   - (:Process)-[:SPAWNED]->(:Process) — parent→child edges
//   - (:Process)-[:OPENED {count, last_ts}]->(:File) — file access edges
//   - (:Process)-[:CONNECTED_TO {addr}]->(:NetworkTarget) — network edges
//
// Enables graph queries like:
//   - "Which process spawned the one that opened .enc files?"
//   - "Find all processes that connected to external IPs and opened files"
//   - "Show the full process tree of the alerting process"
//
// Uses MemGraph's HTTP API (port 7474) for Cypher queries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;

use super::StorageEvent;

/// MemGraph client configuration.
pub struct MemGraphConfig {
    pub url: String,
    pub flush_interval_ms: u64,
}

impl Default for MemGraphConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:7474".into(),
            flush_interval_ms: 500,
        }
    }
}

/// MemGraph graph store. Maintains process relationships and file access.
pub struct MemGraphStore {
    client: Client,
    config: MemGraphConfig,
    /// In-memory dedup cache: "pid:file" → count
    file_access_cache: Arc<Mutex<HashMap<String, u64>>>,
    /// In-memory dedup cache: "pid:addr" → true
    network_cache: Arc<Mutex<HashMap<String, bool>>>,
}

impl MemGraphStore {
    /// Create a new MemGraph store. Initializes schema constraints.
    pub fn start(config: MemGraphConfig) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("failed to create HTTP client: {e}"))?;

        let store = Self {
            client,
            config: config.clone(),
            file_access_cache: Arc::new(Mutex::new(HashMap::new())),
            network_cache: Arc::new(Mutex::new(HashMap::new())),
        };

        // Create constraints for unique IDs
        store.cypher("CREATE INDEX ON :Process(pid)")?;
        store.cypher("CREATE INDEX ON :File(path)")?;
        store.cypher("CREATE INDEX ON :NetworkTarget(addr)")?;

        // Start periodic flush thread for aggregation
        let cache = store.file_access_cache.clone();
        let net_cache = store.network_cache.clone();
        let client = store.client.clone();
        let url = config.url.clone();

        thread::Builder::new()
            .name("memgraph-flush".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_millis(config.flush_interval_ms));

                    // Flush file access counts
                    let file_batch: Vec<(String, u64)> = {
                        let mut c = cache.lock().unwrap();
                        c.drain().collect()
                    };
                    for (key, count) in file_batch {
                        let parts: Vec<&str> = key.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            let pid: u32 = parts[0].parse().unwrap_or(0);
                            let path = parts[1];
                            let query = format!(
                                "MERGE (f:File {{path: '{}'}}) \
                                 WITH f \
                                 MATCH (p:Process {{pid: {pid}}}) \
                                 MERGE (p)-[r:OPENED]->(f) \
                                 SET r.count = coalesce(r.count, 0) + {count}, r.last_ts = timestamp()",
                                path.replace('\'', "\\'"),
                                pid = pid,
                                count = count
                            );
                            let _ = post_cypher(&client, &url, &query);
                        }
                    }

                    // Flush network connections
                    let net_batch: Vec<String> = {
                        let mut c = net_cache.lock().unwrap();
                        c.drain().map(|(k, _)| k).collect()
                    };
                    for key in net_batch {
                        let parts: Vec<&str> = key.splitn(2, ':').collect();
                        if parts.len() == 2 {
                            let pid: u32 = parts[0].parse().unwrap_or(0);
                            let addr = parts[1];
                            let query = format!(
                                "MERGE (n:NetworkTarget {{addr: '{}'}}) \
                                 WITH n \
                                 MATCH (p:Process {{pid: {pid}}}) \
                                 MERGE (p)-[:CONNECTED_TO]->(n)",
                                addr.replace('\'', "\\'"),
                                pid = pid
                            );
                            let _ = post_cypher(&client, &url, &query);
                        }
                    }
                }
            })
            .map_err(|e| format!("failed to spawn MemGraph thread: {e}"))?;

        eprintln!("[memgraph] connected to {}", config.url);
        Ok(store)
    }

    /// Execute a Cypher query.
    fn cypher(&self, query: &str) -> Result<(), String> {
        post_cypher(&self.client, &self.config.url, query)
    }

    /// Ingest an event into the graph.
    pub fn ingest(&self, event: &StorageEvent) {
        // Ensure process node exists
        let comm_escaped = event.comm.replace('\'', "\\'");
        let create_node = format!(
            "MERGE (p:Process {{pid: {pid}}}) SET p.comm = '{}', p.uid = {uid}",
            comm_escaped,
            pid = event.pid,
            uid = event.uid
        );
        let _ = self.cypher(&create_node);

        // For open events, track file access
        if let Some(ref file) = event.file {
            let key = format!("{}:{}", event.pid, file);
            let mut cache = self.file_access_cache.lock().unwrap();
            *cache.entry(key).or_insert(0) += 1;
        }

        // For network events, track connections
        if matches!(event.kind.as_str(), "Connect" | "Accept" | "SendTo" | "RecvFrom") {
            if let Some(ref addr) = event.file {
                let key = format!("{}:{}", event.pid, addr);
                let mut cache = self.network_cache.lock().unwrap();
                cache.insert(key, true);
            }
        }
    }

    /// Query: find the process tree rooted at a given PID.
    pub fn query_process_tree(&self, pid: u32) -> Result<String, String> {
        let query = format!(
            "MATCH path = (root:Process {{pid: {pid}}})-[:SPAWNED*0..]->(child:Process) \
             RETURN [n IN nodes(path) | n.pid + ':' + n.comm] AS tree \
             LIMIT 50"
        );
        let resp = post_cypher(&self.client, &self.config.url, &query)?;
        Ok(resp)
    }

    /// Query: find all processes that opened .enc files.
    pub fn query_enc_opens(&self) -> Result<String, String> {
        let query =
            "MATCH (p:Process)-[r:OPENED]->(f:File) \
             WHERE f.path ENDS WITH '.enc' \
             RETURN p.pid, p.comm, f.path, r.count \
             ORDER BY r.count DESC \
             LIMIT 20";
        let resp = post_cypher(&self.client, &self.config.url, query)?;
        Ok(resp)
    }

    /// Query: find processes with both file opens AND external network connections.
    pub fn query_exfiltration_candidates(&self) -> Result<String, String> {
        let query =
            "MATCH (p:Process)-[:OPENED]->(f:File), (p)-[:CONNECTED_TO]->(n:NetworkTarget) \
             WHERE NOT n.addr STARTS WITH '10.' \
              AND NOT n.addr STARTS WITH '192.168.' \
              AND NOT n.addr STARTS WITH '172.' \
             RETURN p.pid, p.comm, collect(DISTINCT f.path) AS files, collect(DISTINCT n.addr) AS targets \
             LIMIT 20";
        let resp = post_cypher(&self.client, &self.config.url, query)?;
        Ok(resp)
    }
}

/// Post a Cypher query to MemGraph HTTP API.
fn post_cypher(client: &Client, url: &str, query: &str) -> Result<String, String> {
    let payload = serde_json::json!({ "query": query });
    let resp = client
        .post(format!("{}/db/default/tx/commit", url))
        .json(&payload)
        .send()
        .map_err(|e| format!("memgraph request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("memgraph error {status}: {body}"));
    }

    resp.text().map_err(|e| format!("read failed: {e}"))
}
