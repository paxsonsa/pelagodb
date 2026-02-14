//! Background job storage
//!
//! Job state storage at (db, ns, _jobs, job_type, job_id)
//! Jobs created in Pending state - execution deferred to Phase 2

use serde::{Deserialize, Serialize};

/// Job status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// Job record
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub job_id: String,
    pub job_type: String,
    pub status: JobStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Job {
    pub fn new(job_id: impl Into<String>, job_type: impl Into<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as i64;

        Self {
            job_id: job_id.into(),
            job_type: job_type.into(),
            status: JobStatus::Pending,
            created_at: now,
            updated_at: now,
            progress: 0.0,
            error: None,
        }
    }
}
