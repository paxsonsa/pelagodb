use crate::api::{ApiTierStatus, PelagoSite, PelagoSiteStatus, ReplicatorTierStatus};
use crate::resources;
use crate::status::{
    now_rfc3339, set_condition, CONDITION_API_AVAILABLE, CONDITION_CONFIG_VALID,
    CONDITION_DEGRADED, CONDITION_PROGRESSING, CONDITION_READY, CONDITION_REPLICATOR_AVAILABLE,
};
use crate::webhook::{apply_defaults, validate_site};
use anyhow::Result;
use futures::StreamExt;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::core::v1::{ConfigMap, Secret, Service};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, Patch, PatchParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher;
use kube::{Api, Client, Resource, ResourceExt};
use parking_lot::RwLock;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{error, info, warn};

pub type SecretIndex = Arc<RwLock<HashMap<(String, String), HashSet<String>>>>;

#[derive(Clone)]
pub struct OperatorContext {
    pub client: Client,
    pub field_manager: String,
    pub reconcile_interval: Duration,
    pub is_leader: Arc<AtomicBool>,
    pub secret_index: SecretIndex,
}

#[derive(Debug, Error)]
pub enum OperatorError {
    #[error("kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    #[error("missing namespace on {0}")]
    MissingNamespace(String),
    #[error("missing object name")]
    MissingName,
    #[error("missing referenced secret {namespace}/{name}")]
    MissingSecret { namespace: String, name: String },
    #[error("invalid PelagoSite spec: {0}")]
    InvalidSpec(String),
}

pub async fn run_controller(context: OperatorContext, namespace: Option<String>) -> Result<()> {
    let client = context.client.clone();
    let sites: Api<PelagoSite> = match namespace.as_deref() {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };
    let secrets: Api<Secret> = match namespace.as_deref() {
        Some(ns) => Api::namespaced(client.clone(), ns),
        None => Api::all(client.clone()),
    };

    let secret_index = context.secret_index.clone();

    Controller::new(sites, watcher::Config::default())
        .watches(secrets, watcher::Config::default(), move |secret| {
            map_secret_to_sites(secret, &secret_index)
        })
        .run(reconcile, error_policy, Arc::new(context))
        .for_each(|result| async move {
            match result {
                Ok((site_ref, _action)) => {
                    info!("reconciled PelagoSite {}", site_ref.name)
                }
                Err(err) => error!("reconcile failed: {}", err),
            }
        })
        .await;

    Ok(())
}

fn map_secret_to_sites(
    secret: Secret,
    secret_index: &SecretIndex,
) -> Vec<kube::runtime::reflector::ObjectRef<PelagoSite>> {
    let Some(namespace) = secret.namespace() else {
        return Vec::new();
    };

    let key = (namespace.clone(), secret.name_any());
    let index = secret_index.read();
    let Some(site_names) = index.get(&key) else {
        return Vec::new();
    };

    site_names
        .iter()
        .map(|name| kube::runtime::reflector::ObjectRef::new(name).within(&namespace))
        .collect()
}

async fn reconcile(
    site: Arc<PelagoSite>,
    ctx: Arc<OperatorContext>,
) -> Result<Action, OperatorError> {
    if !ctx.is_leader.load(Ordering::Relaxed) {
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    let mut desired = (*site).clone();
    apply_defaults(&mut desired);

    let validation_errors = validate_site(&desired);
    if !validation_errors.is_empty() {
        let mut status = desired.status.clone().unwrap_or_default();
        set_condition(
            &mut status,
            CONDITION_CONFIG_VALID,
            "False",
            "ValidationFailed",
            validation_errors.join("; "),
        );
        set_condition(
            &mut status,
            CONDITION_DEGRADED,
            "True",
            "ValidationFailed",
            "PelagoSite spec validation failed",
        );
        patch_status(&desired, status, &ctx).await?;
        return Err(OperatorError::InvalidSpec(validation_errors.join("; ")));
    }

    let namespace = desired
        .namespace()
        .ok_or_else(|| OperatorError::MissingNamespace(desired.name_any()))?;

    ensure_referenced_secrets(&desired, &ctx.client, &namespace).await?;
    update_secret_index(&desired, &namespace, &ctx.secret_index);

    let cm_api: Api<ConfigMap> = Api::namespaced(ctx.client.clone(), &namespace);
    let svc_api: Api<Service> = Api::namespaced(ctx.client.clone(), &namespace);
    let deploy_api: Api<Deployment> = Api::namespaced(ctx.client.clone(), &namespace);
    let hpa_api: Api<HorizontalPodAutoscaler> = Api::namespaced(ctx.client.clone(), &namespace);
    let pdb_api: Api<PodDisruptionBudget> = Api::namespaced(ctx.client.clone(), &namespace);

    apply(
        &cm_api,
        &resources::build_config_map(&desired),
        &ctx.field_manager,
    )
    .await?;
    apply(
        &svc_api,
        &resources::build_api_service(&desired),
        &ctx.field_manager,
    )
    .await?;
    apply(
        &deploy_api,
        &resources::build_api_deployment(&desired),
        &ctx.field_manager,
    )
    .await?;
    apply(
        &deploy_api,
        &resources::build_replicator_deployment(&desired),
        &ctx.field_manager,
    )
    .await?;

    if let Some(hpa) = resources::build_api_hpa(&desired) {
        apply(&hpa_api, &hpa, &ctx.field_manager).await?;
    } else {
        delete_if_exists(&hpa_api, &resources::api_hpa_name(&desired)).await?;
    }

    if let Some(pdb) = resources::build_api_pdb(&desired) {
        apply(&pdb_api, &pdb, &ctx.field_manager).await?;
    } else {
        delete_if_exists(&pdb_api, &resources::api_pdb_name(&desired)).await?;
    }

    let api_deployment = deploy_api
        .get(&resources::api_deployment_name(&desired))
        .await?;
    let replicator_deployment = deploy_api
        .get(&resources::replicator_deployment_name(&desired))
        .await?;

    let mut status = desired.status.clone().unwrap_or_default();
    status.observed_generation = desired.metadata.generation;
    status.last_reconcile_time = Some(now_rfc3339());
    status.managed_resources = resources::managed_resource_names(&desired);
    status.api = Some(ApiTierStatus {
        ready_replicas: ready_replicas(&api_deployment),
        desired_replicas: desired_replicas(&api_deployment),
        service_name: Some(resources::api_service_name(&desired)),
        hpa_name: if desired.spec.api.hpa.enabled {
            Some(resources::api_hpa_name(&desired))
        } else {
            None
        },
    });
    status.replicator = Some(ReplicatorTierStatus {
        ready_replicas: ready_replicas(&replicator_deployment),
        desired_replicas: desired_replicas(&replicator_deployment),
    });

    let api_ready = status
        .api
        .as_ref()
        .map(|tier| tier.ready_replicas >= tier.desired_replicas)
        .unwrap_or(false);
    let replicator_ready = status
        .replicator
        .as_ref()
        .map(|tier| tier.ready_replicas >= tier.desired_replicas)
        .unwrap_or(false);

    set_condition(
        &mut status,
        CONDITION_CONFIG_VALID,
        "True",
        "Validated",
        "spec accepted",
    );
    set_condition(
        &mut status,
        CONDITION_API_AVAILABLE,
        if api_ready { "True" } else { "False" },
        "DeploymentObserved",
        format!(
            "api readyReplicas={} desiredReplicas={}",
            ready_replicas(&api_deployment),
            desired_replicas(&api_deployment)
        ),
    );
    set_condition(
        &mut status,
        CONDITION_REPLICATOR_AVAILABLE,
        if replicator_ready { "True" } else { "False" },
        "DeploymentObserved",
        format!(
            "replicator readyReplicas={} desiredReplicas={}",
            ready_replicas(&replicator_deployment),
            desired_replicas(&replicator_deployment)
        ),
    );
    set_condition(
        &mut status,
        CONDITION_PROGRESSING,
        if api_ready && replicator_ready {
            "False"
        } else {
            "True"
        },
        "Reconciling",
        "waiting for rollout convergence",
    );
    set_condition(
        &mut status,
        CONDITION_READY,
        if api_ready && replicator_ready {
            "True"
        } else {
            "False"
        },
        "ResourcesApplied",
        "resource reconciliation completed",
    );
    set_condition(
        &mut status,
        CONDITION_DEGRADED,
        "False",
        "Healthy",
        "no errors detected",
    );

    patch_status(&desired, status, &ctx).await?;

    Ok(Action::requeue(ctx.reconcile_interval))
}

fn error_policy(site: Arc<PelagoSite>, err: &OperatorError, _ctx: Arc<OperatorContext>) -> Action {
    warn!("reconcile error for {}: {}", site.name_any(), err);
    Action::requeue(Duration::from_secs(15))
}

async fn ensure_referenced_secrets(
    site: &PelagoSite,
    client: &Client,
    namespace: &str,
) -> Result<(), OperatorError> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), namespace);
    for secret_ref in site.spec.referenced_secret_keys() {
        let exists = secrets.get_opt(&secret_ref.name).await?.is_some();
        if !exists {
            return Err(OperatorError::MissingSecret {
                namespace: namespace.to_string(),
                name: secret_ref.name,
            });
        }
    }
    Ok(())
}

fn update_secret_index(site: &PelagoSite, namespace: &str, index: &SecretIndex) {
    let site_name = site.name_any();
    let mut lock = index.write();

    for refs in lock.values_mut() {
        refs.remove(&site_name);
    }

    for secret_ref in site.spec.referenced_secret_keys() {
        lock.entry((namespace.to_string(), secret_ref.name))
            .or_default()
            .insert(site_name.clone());
    }

    lock.retain(|_, sites| !sites.is_empty());
}

async fn apply<K>(api: &Api<K>, desired: &K, field_manager: &str) -> Result<(), OperatorError>
where
    K: Clone
        + Serialize
        + serde::de::DeserializeOwned
        + std::fmt::Debug
        + Resource<DynamicType = ()>
        + k8s_openapi::Metadata<Ty = ObjectMeta>,
{
    let name = desired
        .meta()
        .name
        .as_deref()
        .ok_or(OperatorError::MissingName)?;

    api.patch(
        name,
        &PatchParams::apply(field_manager).force(),
        &Patch::Apply(desired.clone()),
    )
    .await?;

    Ok(())
}

async fn delete_if_exists<K>(api: &Api<K>, name: &str) -> Result<(), OperatorError>
where
    K: Clone + serde::de::DeserializeOwned + std::fmt::Debug + Resource<DynamicType = ()>,
{
    match api.delete(name, &DeleteParams::default()).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(err)) if err.code == 404 => Ok(()),
        Err(err) => Err(OperatorError::Kube(err)),
    }
}

async fn patch_status(
    site: &PelagoSite,
    status: PelagoSiteStatus,
    ctx: &OperatorContext,
) -> Result<(), OperatorError> {
    let namespace = site
        .namespace()
        .ok_or_else(|| OperatorError::MissingNamespace(site.name_any()))?;
    let api: Api<PelagoSite> = Api::namespaced(ctx.client.clone(), &namespace);
    let name = site.name_any();

    api.patch_status(
        &name,
        &PatchParams::apply(&ctx.field_manager).force(),
        &Patch::Merge(serde_json::json!({ "status": status })),
    )
    .await?;

    Ok(())
}

fn ready_replicas(deployment: &Deployment) -> i32 {
    deployment
        .status
        .as_ref()
        .and_then(|status| status.ready_replicas)
        .unwrap_or(0)
}

fn desired_replicas(deployment: &Deployment) -> i32 {
    deployment
        .spec
        .as_ref()
        .and_then(|spec| spec.replicas)
        .unwrap_or(0)
}
