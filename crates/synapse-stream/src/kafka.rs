// Kafka-wire output: serialize ChangeEvent → JSON → produce to rskafka topic.
// Gate: feature = "kafka-wire"

use anyhow::Result;
use rskafka::{
    client::{Client, ClientBuilder, partition::{Compression, OffsetAt, UnknownTopicHandling}},
    record::Record,
};
use crate::cdc::ChangeEvent;
use chrono::Utc;
use std::sync::Arc;

pub struct KafkaSink {
    client: Arc<Client>,
    topic: String,
}

impl KafkaSink {
    pub async fn new(brokers: Vec<String>, topic: impl Into<String>) -> Result<Self> {
        let client = ClientBuilder::new(brokers).build().await?;
        Ok(Self { client: Arc::new(client), topic: topic.into() })
    }

    pub async fn send(&self, ev: &ChangeEvent) -> Result<()> {
        let payload = serde_json::to_vec(ev)?;
        let controller = self.client.controller_client()?;
        // Create topic if missing (best-effort).
        let _ = controller.create_topic(&self.topic, 1, 1, 5_000).await;

        let partition = self.client
            .partition_client(&self.topic, 0, UnknownTopicHandling::Retry)
            .await?;

        partition.produce(
            vec![Record {
                key: None,
                value: Some(payload),
                headers: Default::default(),
                timestamp: Utc::now(),
            }],
            Compression::NoCompression,
        ).await?;
        Ok(())
    }
}
