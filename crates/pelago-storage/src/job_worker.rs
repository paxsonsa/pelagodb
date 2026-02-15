//! Job worker loop
//!
//! The worker polls for actionable jobs (Pending or Running) and executes
//! them in batches. On startup it recovers any Running jobs that were
//! interrupted (e.g. due to a crash) by resuming from their saved cursor.
//!
//! The worker runs until a shutdown signal is received, at which point it
//! saves the current job state and exits gracefully.

use crate::db::PelagoDb;
use crate::job_executor::executor_for_job;
use crate::jobs::{JobState, JobStatus, JobStore};
use crate::schema::SchemaRegistry;
use pelago_core::PelagoError;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Background job worker
pub struct JobWorker {
    db: PelagoDb,
    job_store: JobStore,
    schema_registry: Arc<SchemaRegistry>,
    poll_interval: Duration,
    batch_size: usize,
}

impl JobWorker {
    pub fn new(db: PelagoDb, schema_registry: Arc<SchemaRegistry>) -> Self {
        Self {
            job_store: JobStore::new(db.clone()),
            db,
            schema_registry,
            poll_interval: Duration::from_secs(5),
            batch_size: 1000,
        }
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Run the worker loop until shutdown signal.
    ///
    /// On startup, recovers any Running jobs (which were interrupted).
    /// Then continuously polls for new Pending jobs.
    pub async fn run(
        &self,
        database: &str,
        namespace: &str,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), PelagoError> {
        info!("Job worker starting for {}/{}", database, namespace);

        loop {
            // Check for shutdown
            if *shutdown.borrow() {
                info!("Job worker received shutdown signal");
                break;
            }

            // Find actionable jobs
            let jobs = self
                .job_store
                .find_actionable_jobs(database, namespace)
                .await?;

            if jobs.is_empty() {
                // Wait before polling again, but check for shutdown
                tokio::select! {
                    _ = tokio::time::sleep(self.poll_interval) => {},
                    _ = shutdown.changed() => {
                        info!("Job worker received shutdown signal during sleep");
                        break;
                    }
                }
                continue;
            }

            for mut job in jobs {
                // Check shutdown before each job
                if *shutdown.borrow() {
                    break;
                }

                info!(
                    job_id = %job.job_id,
                    job_type = %job.job_type.name(),
                    status = ?job.status,
                    "Executing job"
                );

                if let Err(e) = self.execute_job(&mut job).await {
                    warn!(
                        job_id = %job.job_id,
                        error = %e,
                        "Job execution failed"
                    );
                }
            }
        }

        info!("Job worker shutdown complete");
        Ok(())
    }

    /// Execute a single job to completion (or failure)
    async fn execute_job(&self, job: &mut JobState) -> Result<(), PelagoError> {
        let executor = executor_for_job(job, Arc::clone(&self.schema_registry))?;

        job.status = JobStatus::Running;
        self.job_store.update_job(job).await?;

        loop {
            match executor.execute_batch(job, &self.db, self.batch_size).await {
                Ok(true) => {
                    // More work — save progress periodically
                    if job.processed_items % 10000 == 0 {
                        self.job_store.update_job(job).await?;
                    }
                }
                Ok(false) => {
                    // Done
                    job.status = JobStatus::Completed;
                    job.progress_pct = 1.0;
                    self.job_store.update_job(job).await?;
                    info!(
                        job_id = %job.job_id,
                        processed = job.processed_items,
                        "Job completed"
                    );
                    return Ok(());
                }
                Err(e) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(e.to_string());
                    self.job_store.update_job(job).await?;
                    return Err(e);
                }
            }
        }
    }
}
