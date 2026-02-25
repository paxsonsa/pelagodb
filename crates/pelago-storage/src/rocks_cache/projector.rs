use super::config::RocksCacheConfig;
use super::store::RocksCacheStore;
use crate::cdc::CdcOperation;
use crate::cdc::Versionstamp;
use crate::consumer::{CdcConsumer, ConsumerConfig};
use crate::db::PelagoDb;
use pelago_core::PelagoError;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

pub struct CdcProjector {
    db: PelagoDb,
    consumer: CdcConsumer,
    cache: Arc<RocksCacheStore>,
    #[allow(dead_code)]
    batch_size: usize,
    database: String,
    namespace: String,
}

pub struct RebuildStats {
    pub entries_processed: usize,
    pub nodes_written: usize,
    pub edges_written: usize,
}

impl CdcProjector {
    pub async fn new(
        db: PelagoDb,
        cache: Arc<RocksCacheStore>,
        config: &RocksCacheConfig,
        database: &str,
        namespace: &str,
    ) -> Result<Self, PelagoError> {
        let consumer_id = format!(
            "cache_projector_{}_{}_{}_{}",
            config.site_id,
            database,
            namespace,
            projector_instance_id()
        );
        let consumer_config = ConsumerConfig::new(&consumer_id, database, namespace)
            .with_batch_size(config.projector_batch_size);
        let consumer = CdcConsumer::new(db.clone(), consumer_config).await?;

        Ok(Self {
            db,
            consumer,
            cache,
            batch_size: config.projector_batch_size,
            database: database.to_string(),
            namespace: namespace.to_string(),
        })
    }

    pub async fn run(
        &mut self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), PelagoError> {
        info!(
            "CDC projector started for {}/{}",
            self.database, self.namespace
        );
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("CDC projector shutting down");
                        self.consumer.checkpoint().await?;
                        return Ok(());
                    }
                }
                result = self.project_batch() => {
                    match result {
                        Ok(0) => {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        Ok(n) => {
                            debug!("Projected {} CDC entries", n);
                        }
                        Err(e) => {
                            warn!("CDC projection error: {}", e);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
    }

    pub async fn project_batch(&mut self) -> Result<usize, PelagoError> {
        let entries = self.consumer.poll_batch().await?;
        let count = entries.len();

        for (vs, entry) in &entries {
            self.cache.apply_cdc_operations(
                &self.database,
                &self.namespace,
                &entry.operations,
                entry.timestamp,
                vs,
            )?;
        }

        self.consumer.ack_batch(&entries).await?;

        Ok(count)
    }

    pub async fn rebuild(&mut self) -> Result<RebuildStats, PelagoError> {
        info!("Starting cache rebuild from CDC");
        self.cache.clear()?;

        let replay_consumer_id = format!(
            "cache_projector_rebuild_{}_{}",
            self.database,
            std::process::id()
        );
        let replay_config =
            ConsumerConfig::new(&replay_consumer_id, &self.database, &self.namespace)
                .with_batch_size(self.batch_size);
        let mut replay_consumer = CdcConsumer::new(self.db.clone(), replay_config).await?;

        let mut stats = RebuildStats {
            entries_processed: 0,
            nodes_written: 0,
            edges_written: 0,
        };

        loop {
            let entries = replay_consumer.poll_batch().await?;
            if entries.is_empty() {
                break;
            }

            for (vs, entry) in &entries {
                self.cache.apply_cdc_operations(
                    &self.database,
                    &self.namespace,
                    &entry.operations,
                    entry.timestamp,
                    vs,
                )?;
                stats.entries_processed += 1;

                for op in &entry.operations {
                    match op {
                        CdcOperation::NodeCreate { .. }
                        | CdcOperation::NodeUpdate { .. }
                        | CdcOperation::NodeDelete { .. }
                        | CdcOperation::OwnershipTransfer { .. } => stats.nodes_written += 1,
                        CdcOperation::EdgeCreate { .. } | CdcOperation::EdgeDelete { .. } => {
                            stats.edges_written += 1
                        }
                        CdcOperation::SchemaRegister { .. } => {}
                    }
                }
            }

            replay_consumer.ack_batch(&entries).await?;
        }

        info!(
            "Cache rebuild complete: {} entries processed",
            stats.entries_processed
        );
        Ok(stats)
    }

    pub fn hwm(&self) -> &Versionstamp {
        self.consumer.hwm()
    }
}

fn projector_instance_id() -> String {
    std::env::var("POD_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| format!("pid{}", std::process::id()))
}
