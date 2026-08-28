// ── storage/mod.rs — Event storage backends ──────────────────────────────
//
// Provides pluggable storage backends for the event pipeline:
//   - Kafka:      event streaming to Kafka topics
//   - ClickHouse: columnar analytics storage
//   - MemGraph:   process relationship graph (Cypher)
//
// Each backend is feature-gated:
//   --features kafka      → Kafka producer
//   --features clickhouse → ClickHouse inserter
//   --features memgraph   → MemGraph Cypher client

#[cfg(feature = "kafka")]
pub mod kafka;

#[cfg(feature = "clickhouse")]
pub mod clickhouse;

#[cfg(feature = "memgraph")]
pub mod memgraph;

use crate::monitor::{RecordedEvent, Alert};

/// Unified event type for storage backends.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageEvent {
    pub ts: String,
    pub kind: String,
    pub pid: u32,
    pub uid: u32,
    pub comm: String,
    pub file: Option<String>,
    pub extension: Option<String>,
    pub argv: Option<String>,
}

impl From<&RecordedEvent> for StorageEvent {
    fn from(ev: &RecordedEvent) -> Self {
        Self {
            ts: ev.ts.clone(),
            kind: format!("{:?}", ev.kind),
            pid: ev.pid,
            uid: ev.uid,
            comm: ev.comm.clone(),
            file: ev.file.clone(),
            extension: ev.extension.clone(),
            argv: ev.argv.clone(),
        }
    }
}

impl From<&Alert> for StorageEvent {
    fn from(al: &Alert) -> Self {
        Self {
            ts: al.ts.clone(),
            kind: "Alert".into(),
            pid: al.pid,
            uid: al.uid,
            comm: al.comm.clone(),
            file: None,
            extension: None,
            argv: None,
        }
    }
}

// ── Unified pipeline ─────────────────────────────────────────────────────

/// Unified storage pipeline that wraps all active backends.
/// Methods are always compiled — internally gated by #[cfg].
pub struct StoragePipeline {
    #[cfg(feature = "kafka")]
    pub kafka: Option<kafka::KafkaProducer>,
    #[cfg(feature = "clickhouse")]
    pub clickhouse: Option<clickhouse::ClickHouseStore>,
    #[cfg(feature = "memgraph")]
    pub memgraph: Option<memgraph::MemGraphStore>,
}

impl StoragePipeline {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "kafka")]
            kafka: None,
            #[cfg(feature = "clickhouse")]
            clickhouse: None,
            #[cfg(feature = "memgraph")]
            memgraph: None,
        }
    }

    /// Forward an event to all active storage backends.
    pub fn forward_event(&self, _event: &StorageEvent) {
        #[cfg(feature = "kafka")]
        if let Some(ref p) = self.kafka {
            p.send(event.clone());
        }
        #[cfg(feature = "clickhouse")]
        if let Some(ref s) = self.clickhouse {
            s.insert(event.clone());
        }
        #[cfg(feature = "memgraph")]
        if let Some(ref g) = self.memgraph {
            g.ingest(event);
        }
    }

    /// Forward a list of outputs to all active backends.
    pub fn forward_outputs(&self, outputs: &[crate::monitor::Output]) {
        for output in outputs {
            let ev = match output {
                crate::monitor::Output::Event(ref e) => StorageEvent::from(e),
                crate::monitor::Output::Alert(ref a) => StorageEvent::from(a),
                crate::monitor::Output::Action(_) => continue,
            };
            self.forward_event(&ev);
        }
    }
}
