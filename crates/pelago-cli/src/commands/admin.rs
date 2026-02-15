use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    DropEntityTypeRequest, DropNamespaceRequest, GetJobStatusRequest, Job, JobStatus,
    ListJobsRequest, RequestContext,
};

#[derive(clap::Args)]
pub struct AdminArgs {
    #[command(subcommand)]
    pub command: AdminCommand,
}

#[derive(clap::Subcommand)]
pub enum AdminCommand {
    /// Job management
    Job(JobArgs),
    /// Drop all data for an entity type
    DropType {
        /// Entity type to drop
        entity_type: String,
    },
    /// Drop an entire namespace
    DropNamespace {
        /// Namespace to drop
        namespace: String,
    },
}

#[derive(clap::Args)]
pub struct JobArgs {
    #[command(subcommand)]
    pub command: JobCommand,
}

#[derive(clap::Subcommand)]
pub enum JobCommand {
    /// Get job status
    Status {
        /// Job ID
        job_id: String,
    },
    /// List all jobs
    List,
}

pub async fn run(
    args: AdminArgs,
    server: &str,
    database: &str,
    namespace: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = GrpcConnection::connect(server).await?;
    let context = RequestContext {
        database: database.to_string(),
        namespace: namespace.to_string(),
        site_id: String::new(),
        request_id: String::new(),
    };

    match args.command {
        AdminCommand::Job(job_args) => match job_args.command {
            JobCommand::Status { job_id } => {
                let mut client = conn.admin_client();
                let resp = client
                    .get_job_status(GetJobStatusRequest {
                        context: Some(context),
                        job_id,
                    })
                    .await?
                    .into_inner();

                if let Some(job) = resp.job {
                    format_job(&job, format);
                } else {
                    println!("Job not found");
                }
            }
            JobCommand::List => {
                let mut client = conn.admin_client();
                let resp = client
                    .list_jobs(ListJobsRequest {
                        context: Some(context),
                    })
                    .await?
                    .into_inner();
                format_jobs(&resp.jobs, format);
            }
        },
        AdminCommand::DropType { entity_type } => {
            let mut client = conn.admin_client();
            let resp = client
                .drop_entity_type(DropEntityTypeRequest {
                    context: Some(context),
                    entity_type: entity_type.clone(),
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "entity_type": entity_type,
                        "cleanup_job_id": resp.cleanup_job_id,
                    }));
                }
                _ => {
                    println!("Dropped entity type: {}", entity_type);
                    if !resp.cleanup_job_id.is_empty() {
                        println!("Cleanup job: {}", resp.cleanup_job_id);
                    }
                }
            }
        }
        AdminCommand::DropNamespace { namespace: ns } => {
            // Override the context namespace with the one being dropped
            let drop_context = RequestContext {
                database: database.to_string(),
                namespace: ns.clone(),
                site_id: String::new(),
                request_id: String::new(),
            };

            let mut client = conn.admin_client();
            let resp = client
                .drop_namespace(DropNamespaceRequest {
                    context: Some(drop_context),
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "namespace": ns,
                        "dropped": resp.dropped,
                    }));
                }
                _ => {
                    if resp.dropped {
                        println!("Namespace '{}' dropped", ns);
                    } else {
                        println!("Namespace '{}' was not found or already dropped", ns);
                    }
                }
            }
        }
    }
    Ok(())
}

fn job_status_name(status: i32) -> &'static str {
    match JobStatus::try_from(status) {
        Ok(JobStatus::Pending) => "pending",
        Ok(JobStatus::Running) => "running",
        Ok(JobStatus::Completed) => "completed",
        Ok(JobStatus::Failed) => "failed",
        _ => "unknown",
    }
}

fn job_to_json(job: &Job) -> serde_json::Value {
    serde_json::json!({
        "job_id": job.job_id,
        "job_type": job.job_type,
        "status": job_status_name(job.status),
        "progress": job.progress,
        "created_at": job.created_at,
        "updated_at": job.updated_at,
        "error": if job.error.is_empty() { None } else { Some(&job.error) },
    })
}

fn format_job(job: &Job, format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            crate::output::print_json(&job_to_json(job));
        }
        OutputFormat::Table => {
            let headers = vec!["Job ID", "Type", "Status", "Progress", "Error"];
            let error_str = if job.error.is_empty() {
                "-".to_string()
            } else {
                job.error.clone()
            };
            let rows = vec![vec![
                job.job_id.clone(),
                job.job_type.clone(),
                job_status_name(job.status).to_string(),
                format!("{:.0}%", job.progress * 100.0),
                error_str,
            ]];
            crate::output::print_table(&headers, &rows);
        }
        OutputFormat::Csv => {
            let headers = vec!["Job ID", "Type", "Status", "Progress", "Error"];
            let rows = vec![vec![
                job.job_id.clone(),
                job.job_type.clone(),
                job_status_name(job.status).to_string(),
                format!("{:.0}%", job.progress * 100.0),
                job.error.clone(),
            ]];
            crate::output::print_csv(&headers, &rows);
        }
    }
}

fn format_jobs(jobs: &[Job], format: &OutputFormat) {
    match format {
        OutputFormat::Json => {
            let json: Vec<serde_json::Value> = jobs.iter().map(job_to_json).collect();
            crate::output::print_json(&serde_json::Value::Array(json));
        }
        OutputFormat::Table | OutputFormat::Csv => {
            let headers = vec!["Job ID", "Type", "Status", "Progress", "Error"];
            let rows: Vec<Vec<String>> = jobs
                .iter()
                .map(|job| {
                    vec![
                        job.job_id.clone(),
                        job.job_type.clone(),
                        job_status_name(job.status).to_string(),
                        format!("{:.0}%", job.progress * 100.0),
                        if job.error.is_empty() {
                            "-".to_string()
                        } else {
                            job.error.clone()
                        },
                    ]
                })
                .collect();
            match format {
                OutputFormat::Table => crate::output::print_table(&headers, &rows),
                OutputFormat::Csv => crate::output::print_csv(&headers, &rows),
                _ => unreachable!(),
            }
        }
    }
}
