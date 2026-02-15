//! Background job storage and types
//!
//! Jobs are stored at `(db, ns, _jobs, job_id)` as CBOR-encoded `JobState`.
//! The worker loop polls for Pending and Running (interrupted) jobs and
//! executes them in batches with cursor-based resumption.
//!
//! Job types:
//! - `IndexBackfill` — populate index entries for existing nodes
//! - `StripProperty` — remove a property from all nodes of a type
//! - `CdcRetention` — delete expired CDC entries
//! - `OrphanedEdgeCleanup` — remove edges to deleted nodes

use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::schema::IndexType;
use pelago_core::PelagoError;
use serde::{Deserialize, Serialize};

/// Job type with parameters
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JobType {
    IndexBackfill {
        entity_type: String,
        property_name: String,
        index_type: IndexType,
    },
    StripProperty {
        entity_type: String,
        property_name: String,
    },
    CdcRetention {
        max_retention_secs: u64,
        checkpoint_aware: bool,
    },
    OrphanedEdgeCleanup {
        deleted_entity_type: String,
    },
}

impl JobType {
    /// Human-readable name for logging
    pub fn name(&self) -> &str {
        match self {
            JobType::IndexBackfill { .. } => "IndexBackfill",
            JobType::StripProperty { .. } => "StripProperty",
            JobType::CdcRetention { .. } => "CdcRetention",
            JobType::OrphanedEdgeCleanup { .. } => "OrphanedEdgeCleanup",
        }
    }
}

/// Job status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// Full job state with cursor-based resumption
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobState {
    pub job_id: String,
    pub job_type: JobType,
    pub status: JobStatus,
    /// Opaque cursor for batch resumption — the next key to process
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_cursor: Option<Vec<u8>>,
    /// Fraction complete (0.0 to 1.0)
    pub progress_pct: f64,
    /// Total items to process (if known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_items: Option<u64>,
    /// Items processed so far
    pub processed_items: u64,
    pub created_at: i64,
    pub updated_at: i64,
    /// Database this job operates on
    pub database: String,
    /// Namespace this job operates on
    pub namespace: String,
    /// Error message if Failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
}

/// Job store for CRUD on job records in FDB
pub struct JobStore {
    db: PelagoDb,
}

impl JobStore {
    pub fn new(db: PelagoDb) -> Self {
        Self { db }
    }

    /// Build the FDB key for a job
    fn job_key(database: &str, namespace: &str, job_id: &str) -> Vec<u8> {
        Subspace::namespace(database, namespace)
            .jobs()
            .pack()
            .add_string(job_id)
            .build()
            .to_vec()
    }

    /// Create a new job in Pending status
    pub async fn create_job(
        &self,
        database: &str,
        namespace: &str,
        job_type: JobType,
    ) -> Result<JobState, PelagoError> {
        let job_id = uuid::Uuid::now_v7().to_string();
        let now = now_micros();

        let job = JobState {
            job_id: job_id.clone(),
            job_type,
            status: JobStatus::Pending,
            progress_cursor: None,
            progress_pct: 0.0,
            total_items: None,
            processed_items: 0,
            created_at: now,
            updated_at: now,
            database: database.to_string(),
            namespace: namespace.to_string(),
            error: None,
        };

        let key = Self::job_key(database, namespace, &job_id);
        let value = encode_cbor(&job)?;
        self.db.set(&key, &value).await?;

        Ok(job)
    }

    /// Update job state (status, cursor, progress)
    pub async fn update_job(&self, job: &JobState) -> Result<(), PelagoError> {
        let key = Self::job_key(&job.database, &job.namespace, &job.job_id);
        // Update the timestamp
        let mut job = job.clone();
        job.updated_at = now_micros();
        let value = encode_cbor(&job)?;
        self.db.set(&key, &value).await
    }

    /// Get job by ID
    pub async fn get_job(
        &self,
        database: &str,
        namespace: &str,
        job_id: &str,
    ) -> Result<Option<JobState>, PelagoError> {
        let key = Self::job_key(database, namespace, job_id);
        match self.db.get(&key).await? {
            Some(bytes) => Ok(Some(decode_cbor(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Find all Pending or Running jobs (for recovery and scheduling)
    pub async fn find_actionable_jobs(
        &self,
        database: &str,
        namespace: &str,
    ) -> Result<Vec<JobState>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).jobs();
        let range_start = subspace.prefix().to_vec();
        let range_end = subspace.range_end().to_vec();

        let results = self.db.get_range(&range_start, &range_end, 1000).await?;

        let mut jobs = Vec::new();
        for (_key, value) in results {
            let job: JobState = decode_cbor(&value)?;
            if matches!(job.status, JobStatus::Pending | JobStatus::Running) {
                jobs.push(job);
            }
        }

        Ok(jobs)
    }

    /// List all jobs for a namespace
    pub async fn list_jobs(
        &self,
        database: &str,
        namespace: &str,
    ) -> Result<Vec<JobState>, PelagoError> {
        let subspace = Subspace::namespace(database, namespace).jobs();
        let range_start = subspace.prefix().to_vec();
        let range_end = subspace.range_end().to_vec();

        let results = self.db.get_range(&range_start, &range_end, 1000).await?;

        let mut jobs = Vec::new();
        for (_key, value) in results {
            let job: JobState = decode_cbor(&value)?;
            jobs.push(job);
        }

        Ok(jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_state_serialization_roundtrip() {
        let job = JobState {
            job_id: "test-123".to_string(),
            job_type: JobType::IndexBackfill {
                entity_type: "User".to_string(),
                property_name: "email".to_string(),
                index_type: IndexType::Unique,
            },
            status: JobStatus::Pending,
            progress_cursor: None,
            progress_pct: 0.0,
            total_items: None,
            processed_items: 0,
            created_at: 1000000,
            updated_at: 1000000,
            database: "testdb".to_string(),
            namespace: "default".to_string(),
            error: None,
        };

        let bytes = encode_cbor(&job).unwrap();
        let decoded: JobState = decode_cbor(&bytes).unwrap();

        assert_eq!(decoded.job_id, "test-123");
        assert_eq!(decoded.status, JobStatus::Pending);
        assert_eq!(decoded.processed_items, 0);
    }

    #[test]
    fn test_job_type_variants_serialize() {
        let types = vec![
            JobType::IndexBackfill {
                entity_type: "T".into(),
                property_name: "p".into(),
                index_type: IndexType::Range,
            },
            JobType::StripProperty {
                entity_type: "T".into(),
                property_name: "old".into(),
            },
            JobType::CdcRetention {
                max_retention_secs: 3600,
                checkpoint_aware: true,
            },
            JobType::OrphanedEdgeCleanup {
                deleted_entity_type: "T".into(),
            },
        ];

        for jt in types {
            let bytes = encode_cbor(&jt).unwrap();
            let decoded: JobType = decode_cbor(&bytes).unwrap();
            assert_eq!(jt.name(), decoded.name());
        }
    }

    #[test]
    fn test_job_status_values() {
        assert_ne!(JobStatus::Pending, JobStatus::Running);
        assert_ne!(JobStatus::Running, JobStatus::Completed);
        assert_ne!(JobStatus::Completed, JobStatus::Failed);
    }

    #[test]
    fn test_job_key_deterministic() {
        let key1 = JobStore::job_key("db", "ns", "job-1");
        let key2 = JobStore::job_key("db", "ns", "job-1");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_different_jobs_different_keys() {
        let key1 = JobStore::job_key("db", "ns", "job-1");
        let key2 = JobStore::job_key("db", "ns", "job-2");
        assert_ne!(key1, key2);
    }
}
