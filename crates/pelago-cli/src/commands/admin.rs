use crate::connection::GrpcConnection;
use crate::output::OutputFormat;
use pelago_proto::{
    DropEntityTypeRequest, DropNamespaceRequest, GetJobStatusRequest, GetNamespaceSettingsRequest,
    GetReplicationStatusRequest, Job, JobStatus, ListJobsRequest, ListSitesRequest,
    QueryAuditLogRequest, RequestContext, SetNamespaceSchemaOwnerRequest,
    TransferNamespaceSchemaOwnerRequest,
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
    /// List registered sites
    Sites,
    /// Show replication status
    ReplicationStatus,
    /// Query audit log
    Audit {
        #[arg(long, default_value = "")]
        principal: String,
        #[arg(long, default_value = "")]
        action: String,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    /// Show namespace settings (including schema owner policy)
    NamespaceSettings,
    /// Set namespace schema owner site
    SetNamespaceOwner {
        /// Claimed site ID or site alias that owns schema changes for this namespace
        site_id: String,
    },
    /// Clear namespace schema owner policy (unrestricted schema updates)
    ClearNamespaceOwner,
    /// Transfer namespace schema owner from expected to target site
    TransferNamespaceOwner {
        /// Current owner site ID or alias expected by caller
        expected_site_id: String,
        /// New owner site ID or alias
        target_site_id: String,
    },
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
        AdminCommand::Sites => {
            let mut client = conn.admin_client();
            let resp = client
                .list_sites(ListSitesRequest {
                    context: Some(context),
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    let json = resp
                        .sites
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "site_id": s.site_id,
                                "site_name": s.site_name,
                                "status": s.status,
                                "claimed_at": s.claimed_at,
                            })
                        })
                        .collect();
                    crate::output::print_json(&serde_json::Value::Array(json));
                }
                OutputFormat::Table | OutputFormat::Csv => {
                    let headers = vec!["Site ID", "Name", "Status", "Claimed At"];
                    let rows: Vec<Vec<String>> = resp
                        .sites
                        .iter()
                        .map(|s| {
                            vec![
                                s.site_id.clone(),
                                s.site_name.clone(),
                                s.status.clone(),
                                s.claimed_at.to_string(),
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
        AdminCommand::ReplicationStatus => {
            let mut client = conn.admin_client();
            let resp = client
                .get_replication_status(GetReplicationStatusRequest {
                    context: Some(context),
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    let json = resp
                        .peers
                        .iter()
                        .map(|p| {
                            serde_json::json!({
                                "remote_site_id": p.remote_site_id,
                                "last_applied_versionstamp": format!("{:x?}", p.last_applied_versionstamp),
                                "lag_events": p.lag_events,
                                "updated_at": p.updated_at,
                            })
                        })
                        .collect();
                    crate::output::print_json(&serde_json::Value::Array(json));
                }
                OutputFormat::Table | OutputFormat::Csv => {
                    let headers = vec!["Remote Site", "Lag Events", "Updated At"];
                    let rows: Vec<Vec<String>> = resp
                        .peers
                        .iter()
                        .map(|p| {
                            vec![
                                p.remote_site_id.clone(),
                                p.lag_events.to_string(),
                                p.updated_at.to_string(),
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
        AdminCommand::Audit {
            principal,
            action,
            limit,
        } => {
            let mut client = conn.admin_client();
            let resp = client
                .query_audit_log(QueryAuditLogRequest {
                    context: Some(context),
                    principal_id: principal,
                    action,
                    from_timestamp: 0,
                    to_timestamp: 0,
                    limit,
                })
                .await?
                .into_inner();

            match format {
                OutputFormat::Json => {
                    let json = resp
                        .events
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "event_id": e.event_id,
                                "principal_id": e.principal_id,
                                "action": e.action,
                                "resource": e.resource,
                                "allowed": e.allowed,
                                "reason": e.reason,
                                "timestamp": e.timestamp,
                                "metadata": e.metadata,
                            })
                        })
                        .collect();
                    crate::output::print_json(&serde_json::Value::Array(json));
                }
                OutputFormat::Table | OutputFormat::Csv => {
                    let headers = vec!["Event ID", "Principal", "Action", "Allowed", "Time"];
                    let rows: Vec<Vec<String>> = resp
                        .events
                        .iter()
                        .map(|e| {
                            vec![
                                e.event_id.clone(),
                                e.principal_id.clone(),
                                e.action.clone(),
                                e.allowed.to_string(),
                                e.timestamp.to_string(),
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
        AdminCommand::NamespaceSettings => {
            let mut client = conn.admin_client();
            let resp = client
                .get_namespace_settings(GetNamespaceSettingsRequest {
                    context: Some(context),
                })
                .await?
                .into_inner();

            let settings = resp.settings.as_ref();
            let owner = settings
                .map(|s| {
                    if s.schema_owner_site_id.is_empty() {
                        "<unrestricted>".to_string()
                    } else {
                        s.schema_owner_site_id.clone()
                    }
                })
                .unwrap_or_else(|| "<unrestricted>".to_string());
            let epoch = settings.map(|s| s.epoch).unwrap_or_default();
            let updated_at = settings.map(|s| s.updated_at).unwrap_or_default();

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "database": database,
                        "namespace": namespace,
                        "configured": resp.found,
                        "schema_owner_site_id": if owner == "<unrestricted>" { serde_json::Value::Null } else { serde_json::Value::String(owner.clone()) },
                        "epoch": epoch,
                        "updated_at": updated_at,
                    }));
                }
                OutputFormat::Table | OutputFormat::Csv => {
                    let headers = vec![
                        "Database",
                        "Namespace",
                        "Configured",
                        "Schema Owner",
                        "Epoch",
                        "Updated At",
                    ];
                    let rows = vec![vec![
                        database.to_string(),
                        namespace.to_string(),
                        resp.found.to_string(),
                        owner,
                        epoch.to_string(),
                        updated_at.to_string(),
                    ]];
                    match format {
                        OutputFormat::Table => crate::output::print_table(&headers, &rows),
                        OutputFormat::Csv => crate::output::print_csv(&headers, &rows),
                        _ => unreachable!(),
                    }
                }
            }
        }
        AdminCommand::SetNamespaceOwner { site_id } => {
            let mut client = conn.admin_client();
            let resp = client
                .set_namespace_schema_owner(SetNamespaceSchemaOwnerRequest {
                    context: Some(context),
                    site_id: site_id.clone(),
                })
                .await?
                .into_inner();
            let settings = resp.settings.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing settings in set-namespace-owner response",
                )
            })?;

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "database": settings.database,
                        "namespace": settings.namespace,
                        "schema_owner_site_id": settings.schema_owner_site_id,
                        "epoch": settings.epoch,
                        "updated_at": settings.updated_at,
                    }));
                }
                _ => {
                    println!(
                        "Namespace owner set: {}/{} -> {} (epoch {})",
                        settings.database,
                        settings.namespace,
                        settings.schema_owner_site_id,
                        settings.epoch
                    );
                }
            }
        }
        AdminCommand::ClearNamespaceOwner => {
            let mut client = conn.admin_client();
            let resp = client
                .set_namespace_schema_owner(SetNamespaceSchemaOwnerRequest {
                    context: Some(context),
                    site_id: String::new(),
                })
                .await?
                .into_inner();
            let settings = resp.settings.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing settings in clear-namespace-owner response",
                )
            })?;

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "database": settings.database,
                        "namespace": settings.namespace,
                        "schema_owner_site_id": serde_json::Value::Null,
                        "epoch": settings.epoch,
                        "updated_at": settings.updated_at,
                    }));
                }
                _ => {
                    println!(
                        "Namespace owner cleared: {}/{} (epoch {})",
                        settings.database, settings.namespace, settings.epoch
                    );
                }
            }
        }
        AdminCommand::TransferNamespaceOwner {
            expected_site_id,
            target_site_id,
        } => {
            let mut client = conn.admin_client();
            let resp = client
                .transfer_namespace_schema_owner(TransferNamespaceSchemaOwnerRequest {
                    context: Some(context),
                    expected_site_id,
                    target_site_id,
                })
                .await?
                .into_inner();
            let settings = resp.settings.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing settings in transfer-namespace-owner response",
                )
            })?;

            match format {
                OutputFormat::Json => {
                    crate::output::print_json(&serde_json::json!({
                        "database": settings.database,
                        "namespace": settings.namespace,
                        "schema_owner_site_id": settings.schema_owner_site_id,
                        "epoch": settings.epoch,
                        "updated_at": settings.updated_at,
                    }));
                }
                _ => {
                    println!(
                        "Namespace owner transferred: {}/{} -> {} (epoch {})",
                        settings.database,
                        settings.namespace,
                        settings.schema_owner_site_id,
                        settings.epoch
                    );
                }
            }
        }
        AdminCommand::DropType { entity_type } => {
            let mut client = conn.admin_client();
            let resp = client
                .drop_entity_type(DropEntityTypeRequest {
                    context: Some(context),
                    entity_type: entity_type.clone(),
                    mutation_mode: pelago_proto::MutationExecutionMode::AsyncAllowed as i32,
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
