// ── storage/kafka.rs — Kafka event producer ──────────────────────────────
//
// Sends detection events to a Kafka topic for downstream consumers
// (ClickHouse, SIEM, dashboards, alerting pipelines).
//
// Uses rdkafka (librdkafka wrapper) with configurable:
//   - Broker address (default: localhost:9092)
//   - Topic name (default: halcyon-events)
//   - Compression (lz4 by default)
//   - Batch size and linger for throughput tuning

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rdkafka::config::ClientConfig;
use rdkafka::producer::{BaseProducer, BaseRecord, Producer};
use rdkafka::util::Timeout;

use super::StorageEvent;

/// Kafka producer configuration.
pub struct KafkaConfig {
    pub brokers: String,
    pub topic: String,
    pub compression: String,
    pub batch_size: usize,
    pub linger_ms: u32,
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".into(),
            topic: "halcyon-events".into(),
            compression: "lz4".into(),
            batch_size: 65536,
            linger_ms: 10,
        }
    }
}

/// Kafka producer handle. Sends events in a background thread.
pub struct KafkaProducer {
    tx: mpsc::Sender<StorageEvent>,
    _thread: thread::JoinHandle<()>,
}

impl KafkaProducer {
    /// Create a new Kafka producer. Connects to the broker and spawns
    /// a background thread for async event delivery.
    pub fn start(config: KafkaConfig) -> Result<Self, String> {
        let mut kafka_config = ClientConfig::new();
        kafka_config
            .set("bootstrap.servers", &config.brokers)
            .set("compression.type", &config.compression)
            .set("batch.size", &config.batch_size.to_string())
            .set("linger.ms", &config.linger_ms.to_string())
            .set("queue.buffering.max.messages", "100000")
            .set("message.send.max.retries", "3")
            .set("retry.backoff.ms", "100");

        let producer: BaseProducer = kafka_config
            .create()
            .map_err(|e| format!("failed to create Kafka producer: {e}"))?;

        let topic = config.topic.clone();
        let (tx, rx) = mpsc::channel::<StorageEvent>();

        let thread = thread::Builder::new()
            .name("kafka-producer".into())
            .spawn(move || {
                // Poll producer to handle delivery callbacks
                let producer_ref = &producer;

                loop {
                    // Drain events from channel
                    while let Ok(event) = rx.try_recv() {
                        let payload = match serde_json::to_string(&event) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };

                        // Use pid as partition key for ordering per-process
                        let key = event.pid.to_string();

                        let record = BaseRecord::to(&topic)
                            .key(&key)
                            .payload(&payload);

                        if let Err((e, _)) = producer_ref.send(record) {
                            eprintln!("[kafka] failed to send event: {e}");
                        }
                    }

                    // Poll to trigger delivery callbacks
                    producer_ref.poll(Duration::from_millis(10));
                }
            })
            .map_err(|e| format!("failed to spawn Kafka thread: {e}"))?;

        eprintln!("[kafka] producer connected to {} topic={}", config.brokers, config.topic);

        Ok(Self { tx, _thread: thread })
    }

    /// Send an event to Kafka (non-blocking).
    pub fn send(&self, event: StorageEvent) {
        let _ = self.tx.send(event);
    }
}

/// Batch send multiple events.
pub fn send_batch(producer: &KafkaProducer, events: &[StorageEvent]) {
    for event in events {
        producer.send(event.clone());
    }
}
