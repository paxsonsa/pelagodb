use anyhow::{Context, Result};
use axum::{routing::get, routing::post, Router};
use kube::Client;
use kube_leader_election::{LeaseLock, LeaseLockParams, LeaseLockResult};
use pelago_operator::controller::{run_controller, OperatorContext, SecretIndex};
use pelago_operator::webhook::{mutate, validate, WebhookState};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let client = Client::try_default()
        .await
        .context("failed to initialize Kubernetes client")?;

    let namespace = std::env::var("OPERATOR_NAMESPACE")
        .ok()
        .and_then(normalize_opt_string);
    let field_manager = std::env::var("PELAGO_OPERATOR_FIELD_MANAGER")
        .unwrap_or_else(|_| "pelago-operator".to_string());
    let reconcile_interval_secs = env_u64("PELAGO_OPERATOR_REQUEUE_SECONDS", 30);

    let is_leader = Arc::new(AtomicBool::new(false));
    let leader_election_enabled = env_bool("PELAGO_OPERATOR_LEADER_ELECTION", true);
    if leader_election_enabled {
        let lease_namespace = std::env::var("PELAGO_OPERATOR_LEASE_NAMESPACE")
            .ok()
            .and_then(normalize_opt_string)
            .or_else(|| namespace.clone())
            .unwrap_or_else(|| "default".to_string());
        let lease_name = std::env::var("PELAGO_OPERATOR_LEASE_NAME")
            .unwrap_or_else(|_| "pelago-operator-lock".to_string());
        let lease_ttl_secs = env_u64("PELAGO_OPERATOR_LEASE_TTL_SECONDS", 15);
        spawn_leader_election(
            client.clone(),
            is_leader.clone(),
            lease_namespace,
            lease_name,
            lease_ttl_secs,
        );
    } else {
        is_leader.store(true, Ordering::Relaxed);
        info!("leader election disabled; controller is always active");
    }

    let webhook_enabled = env_bool("PELAGO_OPERATOR_WEBHOOK_ENABLED", true);
    if webhook_enabled {
        spawn_webhook_server()?;
    }

    let context = OperatorContext {
        client: client.clone(),
        field_manager,
        reconcile_interval: Duration::from_secs(reconcile_interval_secs),
        is_leader,
        secret_index: Arc::new(parking_lot::RwLock::new(HashMap::new())) as SecretIndex,
    };

    run_controller(context, namespace).await
}

fn spawn_leader_election(
    client: Client,
    is_leader: Arc<AtomicBool>,
    lease_namespace: String,
    lease_name: String,
    lease_ttl_secs: u64,
) {
    let holder_id = format!(
        "{}:{}",
        std::env::var("HOSTNAME")
            .ok()
            .or_else(|| std::env::var("HOST").ok())
            .unwrap_or_else(|| "pelago-operator".to_string()),
        std::process::id()
    );

    let renew_interval = Duration::from_secs((lease_ttl_secs / 3).max(1));
    tokio::spawn(async move {
        let lock = LeaseLock::new(
            client,
            &lease_namespace,
            LeaseLockParams {
                lease_name,
                holder_id,
                lease_ttl: Duration::from_secs(lease_ttl_secs.max(3)),
            },
        );

        loop {
            match lock.try_acquire_or_renew().await {
                Ok(result) => {
                    let leading = matches!(result, LeaseLockResult::Acquired(_));
                    is_leader.store(leading, Ordering::Relaxed);
                }
                Err(err) => {
                    error!("leader election renew failed: {}", err);
                    is_leader.store(false, Ordering::Relaxed);
                }
            }
            tokio::time::sleep(renew_interval).await;
        }
    });
}

fn spawn_webhook_server() -> Result<()> {
    let bind_addr = std::env::var("PELAGO_OPERATOR_WEBHOOK_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:9443".to_string());
    let cert_path = std::env::var("PELAGO_OPERATOR_WEBHOOK_CERT")
        .unwrap_or_else(|_| "/tls/tls.crt".to_string());
    let key_path =
        std::env::var("PELAGO_OPERATOR_WEBHOOK_KEY").unwrap_or_else(|_| "/tls/tls.key".to_string());
    let socket: SocketAddr = bind_addr
        .parse()
        .with_context(|| format!("invalid webhook bind address '{}'", bind_addr))?;

    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/mutate", post(mutate))
        .route("/validate", post(validate))
        .with_state(WebhookState::default());

    tokio::spawn(async move {
        let rustls_config = match axum_server::tls_rustls::RustlsConfig::from_pem_file(
            cert_path.clone(),
            key_path.clone(),
        )
        .await
        {
            Ok(config) => config,
            Err(err) => {
                error!(
                    "failed to load webhook TLS keypair cert={} key={}: {}",
                    cert_path, key_path, err
                );
                return;
            }
        };

        info!("starting webhook server on {}", socket);
        if let Err(err) = axum_server::bind_rustls(socket, rustls_config)
            .serve(router.into_make_service())
            .await
        {
            error!("webhook server exited with error: {}", err);
        }
    });

    Ok(())
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(filter)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default)
}

fn normalize_opt_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
