use crate::api::{PelagoSite, ReplicationPeerSpec, ReplicationScopeSpec, SecretKeyRef};
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, DeploymentStrategy};
use k8s_openapi::api::autoscaling::v2::{
    HorizontalPodAutoscaler, HorizontalPodAutoscalerSpec, MetricSpec, MetricTarget,
    ResourceMetricSource,
};
use k8s_openapi::api::core::v1::{
    ConfigMap, ConfigMapEnvSource, Container, ContainerPort, EmptyDirVolumeSource, EnvFromSource,
    EnvVar, EnvVarSource, PodSpec, PodTemplateSpec as K8sPodTemplateSpec, Probe, SecretKeySelector,
    SecretVolumeSource, Service, ServicePort, ServiceSpec, TCPSocketAction, Volume, VolumeMount,
};
use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::{Resource, ResourceExt};
use std::collections::BTreeMap;

pub fn labels(site: &PelagoSite) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("app.kubernetes.io/name".to_string(), "pelagodb".to_string()),
        (
            "app.kubernetes.io/instance".to_string(),
            site.name_any().to_string(),
        ),
        (
            "app.kubernetes.io/managed-by".to_string(),
            "pelago-operator".to_string(),
        ),
    ])
}

pub fn api_selector_labels(site: &PelagoSite) -> BTreeMap<String, String> {
    let mut base = labels(site);
    base.insert("pelago.pelagodb.io/role".to_string(), "api".to_string());
    base
}

pub fn replicator_selector_labels(site: &PelagoSite) -> BTreeMap<String, String> {
    let mut base = labels(site);
    base.insert(
        "pelago.pelagodb.io/role".to_string(),
        "replicator".to_string(),
    );
    base
}

pub fn config_map_name(site: &PelagoSite) -> String {
    format!("{}-config", site.name_any())
}

pub fn api_deployment_name(site: &PelagoSite) -> String {
    format!("{}-api", site.name_any())
}

pub fn replicator_deployment_name(site: &PelagoSite) -> String {
    format!("{}-replicator", site.name_any())
}

pub fn api_service_name(site: &PelagoSite) -> String {
    format!("{}-api", site.name_any())
}

pub fn api_hpa_name(site: &PelagoSite) -> String {
    format!("{}-api", site.name_any())
}

pub fn api_pdb_name(site: &PelagoSite) -> String {
    format!("{}-api", site.name_any())
}

pub fn managed_resource_names(site: &PelagoSite) -> Vec<String> {
    vec![
        format!("ConfigMap/{}", config_map_name(site)),
        format!("Deployment/{}", api_deployment_name(site)),
        format!("Service/{}", api_service_name(site)),
        format!("Deployment/{}", replicator_deployment_name(site)),
        format!("HorizontalPodAutoscaler/{}", api_hpa_name(site)),
        format!("PodDisruptionBudget/{}", api_pdb_name(site)),
    ]
}

pub fn build_config_map(site: &PelagoSite) -> ConfigMap {
    let scope = site
        .spec
        .replicator
        .scopes
        .first()
        .cloned()
        .unwrap_or(ReplicationScopeSpec {
            database: site.spec.defaults.database.clone(),
            namespace: site.spec.defaults.namespace.clone(),
        });

    let scopes_csv = scopes_csv(
        &site.spec.replicator.scopes,
        &site.spec.defaults.database,
        &site.spec.defaults.namespace,
    );

    let data = BTreeMap::from([
        ("PELAGO_SITE_ID".to_string(), site.spec.site.id.to_string()),
        ("PELAGO_SITE_NAME".to_string(), site.spec.site.name.clone()),
        (
            "PELAGO_LISTEN_ADDR".to_string(),
            "0.0.0.0:27615".to_string(),
        ),
        (
            "PELAGO_FDB_CLUSTER".to_string(),
            "/etc/pelago/fdb.cluster".to_string(),
        ),
        (
            "PELAGO_DEFAULT_DATABASE".to_string(),
            site.spec.defaults.database.clone(),
        ),
        (
            "PELAGO_DEFAULT_NAMESPACE".to_string(),
            site.spec.defaults.namespace.clone(),
        ),
        (
            "PELAGO_AUTH_REQUIRED".to_string(),
            site.spec.auth.required.to_string(),
        ),
        ("PELAGO_AUDIT_ENABLED".to_string(), "true".to_string()),
        (
            "PELAGO_CACHE_PROJECTOR_BATCH_SIZE".to_string(),
            site.spec.api.cache.projector_batch_size.to_string(),
        ),
        (
            "PELAGO_REPLICATION_DATABASE".to_string(),
            scope.database.to_string(),
        ),
        (
            "PELAGO_REPLICATION_NAMESPACE".to_string(),
            scope.namespace.to_string(),
        ),
        ("PELAGO_REPLICATION_SCOPES".to_string(), scopes_csv),
        (
            "PELAGO_REPLICATION_BATCH_SIZE".to_string(),
            site.spec.replicator.batch_size.to_string(),
        ),
        (
            "PELAGO_REPLICATION_POLL_MS".to_string(),
            site.spec.replicator.poll_ms.to_string(),
        ),
        (
            "PELAGO_REPLICATION_LEASE_ENABLED".to_string(),
            site.spec.replicator.lease.enabled.to_string(),
        ),
        (
            "PELAGO_REPLICATION_LEASE_TTL_MS".to_string(),
            site.spec.replicator.lease.ttl_ms.to_string(),
        ),
        (
            "PELAGO_REPLICATION_LEASE_HEARTBEAT_MS".to_string(),
            site.spec.replicator.lease.heartbeat_ms.to_string(),
        ),
    ]);

    ConfigMap {
        metadata: metadata(site, &config_map_name(site), labels(site), BTreeMap::new()),
        data: Some(data),
        ..ConfigMap::default()
    }
}

pub fn build_api_service(site: &PelagoSite) -> Service {
    Service {
        metadata: metadata(
            site,
            &api_service_name(site),
            labels(site),
            site.spec.api.service.annotations.clone(),
        ),
        spec: Some(ServiceSpec {
            selector: Some(api_selector_labels(site)),
            type_: Some(
                site.spec
                    .api
                    .service
                    .service_type
                    .as_k8s_value()
                    .to_string(),
            ),
            ports: Some(vec![ServicePort {
                name: Some("grpc".to_string()),
                port: site.spec.api.service.port,
                target_port: Some(
                    k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(
                        "grpc".to_string(),
                    ),
                ),
                ..ServicePort::default()
            }]),
            ..ServiceSpec::default()
        }),
        ..Service::default()
    }
}

pub fn build_api_deployment(site: &PelagoSite) -> Deployment {
    let selector_labels = api_selector_labels(site);
    let mut pod_labels = selector_labels.clone();
    pod_labels.extend(site.spec.pod_template.api.labels.clone());

    let mut container_env = vec![
        EnvVar {
            name: "PELAGO_REPLICATION_ENABLED".to_string(),
            value: Some("false".to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_CACHE_ENABLED".to_string(),
            value: Some(site.spec.api.cache.enabled.to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_CACHE_PATH".to_string(),
            value: Some(site.spec.api.cache.path.clone()),
            ..EnvVar::default()
        },
    ];

    if let Some(secret_ref) = site.spec.auth.api_keys_secret_ref.as_ref() {
        container_env.push(secret_env_var("PELAGO_API_KEYS", secret_ref));
    }
    container_env.extend(site.spec.pod_template.api.extra_env.clone());

    let mut volume_mounts = vec![
        VolumeMount {
            name: "cache".to_string(),
            mount_path: site.spec.api.cache.path.clone(),
            ..VolumeMount::default()
        },
        VolumeMount {
            name: "fdb-cluster".to_string(),
            mount_path: "/etc/pelago".to_string(),
            read_only: Some(true),
            ..VolumeMount::default()
        },
    ];
    volume_mounts.extend(site.spec.pod_template.api.extra_volume_mounts.clone());

    let mut volumes = vec![
        Volume {
            name: "cache".to_string(),
            empty_dir: Some(EmptyDirVolumeSource::default()),
            ..Volume::default()
        },
        secret_volume(
            "fdb-cluster",
            &site.spec.fdb.cluster_file_secret_ref,
            "fdb.cluster",
        ),
    ];
    volumes.extend(site.spec.pod_template.api.extra_volumes.clone());

    Deployment {
        metadata: metadata(
            site,
            &api_deployment_name(site),
            labels(site),
            BTreeMap::new(),
        ),
        spec: Some(DeploymentSpec {
            replicas: Some(site.spec.api.replicas),
            selector: LabelSelector {
                match_labels: Some(selector_labels),
                ..LabelSelector::default()
            },
            strategy: Some(DeploymentStrategy::default()),
            template: K8sPodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(pod_labels),
                    annotations: Some(site.spec.pod_template.api.annotations.clone()),
                    ..ObjectMeta::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "pelagodb-api".to_string(),
                        image: Some(format!(
                            "{}:{}",
                            site.spec.image.repository, site.spec.image.tag
                        )),
                        image_pull_policy: Some(site.spec.image.pull_policy.clone()),
                        ports: Some(vec![ContainerPort {
                            container_port: 27615,
                            name: Some("grpc".to_string()),
                            ..ContainerPort::default()
                        }]),
                        env_from: Some(vec![EnvFromSource {
                            config_map_ref: Some(ConfigMapEnvSource {
                                name: config_map_name(site),
                                optional: Some(false),
                            }),
                            ..EnvFromSource::default()
                        }]),
                        env: Some(container_env),
                        volume_mounts: Some(volume_mounts),
                        readiness_probe: Some(tcp_probe("grpc", 5, 10)),
                        liveness_probe: Some(tcp_probe("grpc", 15, 20)),
                        resources: site.spec.api.resources.clone(),
                        ..Container::default()
                    }],
                    node_selector: if site.spec.pod_template.api.node_selector.is_empty() {
                        None
                    } else {
                        Some(site.spec.pod_template.api.node_selector.clone())
                    },
                    tolerations: if site.spec.pod_template.api.tolerations.is_empty() {
                        None
                    } else {
                        Some(site.spec.pod_template.api.tolerations.clone())
                    },
                    affinity: site.spec.pod_template.api.affinity.clone(),
                    volumes: Some(volumes),
                    ..PodSpec::default()
                }),
            },
            ..DeploymentSpec::default()
        }),
        ..Deployment::default()
    }
}

pub fn build_replicator_deployment(site: &PelagoSite) -> Deployment {
    let selector_labels = replicator_selector_labels(site);
    let mut pod_labels = selector_labels.clone();
    pod_labels.extend(site.spec.pod_template.replicator.labels.clone());

    let peers = peers_csv(&site.spec.replicator.peers);
    let scopes = scopes_csv(
        &site.spec.replicator.scopes,
        &site.spec.defaults.database,
        &site.spec.defaults.namespace,
    );

    let mut env = vec![
        EnvVar {
            name: "PELAGO_REPLICATION_ENABLED".to_string(),
            value: Some(site.spec.replicator.enabled().to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_CACHE_ENABLED".to_string(),
            value: Some("false".to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_PEERS".to_string(),
            value: Some(peers),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_SCOPES".to_string(),
            value: Some(scopes),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_BATCH_SIZE".to_string(),
            value: Some(site.spec.replicator.batch_size.to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_POLL_MS".to_string(),
            value: Some(site.spec.replicator.poll_ms.to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_LEASE_ENABLED".to_string(),
            value: Some(site.spec.replicator.lease.enabled.to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_LEASE_TTL_MS".to_string(),
            value: Some(site.spec.replicator.lease.ttl_ms.to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_LEASE_HEARTBEAT_MS".to_string(),
            value: Some(site.spec.replicator.lease.heartbeat_ms.to_string()),
            ..EnvVar::default()
        },
        EnvVar {
            name: "PELAGO_REPLICATION_TLS_MODE".to_string(),
            value: Some(site.spec.replicator.tls.mode.as_env_value().to_string()),
            ..EnvVar::default()
        },
    ];

    if let Some(secret_ref) = shared_peer_api_key_secret(&site.spec.replicator.peers) {
        env.push(secret_env_var("PELAGO_REPLICATION_API_KEY", &secret_ref));
    }

    if let Some(secret_ref) = site.spec.auth.api_keys_secret_ref.as_ref() {
        env.push(secret_env_var("PELAGO_API_KEYS", secret_ref));
    }

    let mut volume_mounts = vec![VolumeMount {
        name: "fdb-cluster".to_string(),
        mount_path: "/etc/pelago".to_string(),
        read_only: Some(true),
        ..VolumeMount::default()
    }];
    let mut volumes = vec![secret_volume(
        "fdb-cluster",
        &site.spec.fdb.cluster_file_secret_ref,
        "fdb.cluster",
    )];

    if let Some(secret) = site.spec.replicator.tls.ca_bundle_secret_ref.as_ref() {
        let name = "replication-tls-ca";
        volumes.push(secret_volume(name, secret, "ca.pem"));
        volume_mounts.push(VolumeMount {
            name: name.to_string(),
            mount_path: "/etc/pelago/tls/ca".to_string(),
            read_only: Some(true),
            ..VolumeMount::default()
        });
        env.push(EnvVar {
            name: "PELAGO_REPLICATION_TLS_CA".to_string(),
            value: Some("/etc/pelago/tls/ca/ca.pem".to_string()),
            ..EnvVar::default()
        });
    }

    if let Some(secret) = site.spec.replicator.tls.client_cert_secret_ref.as_ref() {
        let name = "replication-tls-cert";
        volumes.push(secret_volume(name, secret, "tls.crt"));
        volume_mounts.push(VolumeMount {
            name: name.to_string(),
            mount_path: "/etc/pelago/tls/cert".to_string(),
            read_only: Some(true),
            ..VolumeMount::default()
        });
        env.push(EnvVar {
            name: "PELAGO_REPLICATION_TLS_CERT".to_string(),
            value: Some("/etc/pelago/tls/cert/tls.crt".to_string()),
            ..EnvVar::default()
        });
    }

    if let Some(secret) = site.spec.replicator.tls.client_key_secret_ref.as_ref() {
        let name = "replication-tls-key";
        volumes.push(secret_volume(name, secret, "tls.key"));
        volume_mounts.push(VolumeMount {
            name: name.to_string(),
            mount_path: "/etc/pelago/tls/key".to_string(),
            read_only: Some(true),
            ..VolumeMount::default()
        });
        env.push(EnvVar {
            name: "PELAGO_REPLICATION_TLS_KEY".to_string(),
            value: Some("/etc/pelago/tls/key/tls.key".to_string()),
            ..EnvVar::default()
        });
    }

    if let Some(server_name) = site.spec.replicator.tls.server_name.as_ref() {
        env.push(EnvVar {
            name: "PELAGO_REPLICATION_TLS_SERVER_NAME".to_string(),
            value: Some(server_name.clone()),
            ..EnvVar::default()
        });
    }

    env.extend(site.spec.pod_template.replicator.extra_env.clone());
    volume_mounts.extend(
        site.spec
            .pod_template
            .replicator
            .extra_volume_mounts
            .clone(),
    );
    volumes.extend(site.spec.pod_template.replicator.extra_volumes.clone());

    Deployment {
        metadata: metadata(
            site,
            &replicator_deployment_name(site),
            labels(site),
            BTreeMap::new(),
        ),
        spec: Some(DeploymentSpec {
            replicas: Some(site.spec.replicator.replicas),
            selector: LabelSelector {
                match_labels: Some(selector_labels),
                ..LabelSelector::default()
            },
            strategy: Some(DeploymentStrategy::default()),
            template: K8sPodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(pod_labels),
                    annotations: Some(site.spec.pod_template.replicator.annotations.clone()),
                    ..ObjectMeta::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "pelagodb-replicator".to_string(),
                        image: Some(format!(
                            "{}:{}",
                            site.spec.image.repository, site.spec.image.tag
                        )),
                        image_pull_policy: Some(site.spec.image.pull_policy.clone()),
                        ports: Some(vec![ContainerPort {
                            container_port: 27615,
                            name: Some("grpc".to_string()),
                            ..ContainerPort::default()
                        }]),
                        env_from: Some(vec![EnvFromSource {
                            config_map_ref: Some(ConfigMapEnvSource {
                                name: config_map_name(site),
                                optional: Some(false),
                            }),
                            ..EnvFromSource::default()
                        }]),
                        env: Some(env),
                        volume_mounts: Some(volume_mounts),
                        resources: site.spec.replicator.resources.clone(),
                        ..Container::default()
                    }],
                    node_selector: if site.spec.pod_template.replicator.node_selector.is_empty() {
                        None
                    } else {
                        Some(site.spec.pod_template.replicator.node_selector.clone())
                    },
                    tolerations: if site.spec.pod_template.replicator.tolerations.is_empty() {
                        None
                    } else {
                        Some(site.spec.pod_template.replicator.tolerations.clone())
                    },
                    affinity: site.spec.pod_template.replicator.affinity.clone(),
                    volumes: Some(volumes),
                    ..PodSpec::default()
                }),
            },
            ..DeploymentSpec::default()
        }),
        ..Deployment::default()
    }
}

pub fn build_api_hpa(site: &PelagoSite) -> Option<HorizontalPodAutoscaler> {
    if !site.spec.api.hpa.enabled {
        return None;
    }

    Some(HorizontalPodAutoscaler {
        metadata: metadata(site, &api_hpa_name(site), labels(site), BTreeMap::new()),
        spec: Some(HorizontalPodAutoscalerSpec {
            scale_target_ref: k8s_openapi::api::autoscaling::v2::CrossVersionObjectReference {
                api_version: Some("apps/v1".to_string()),
                kind: "Deployment".to_string(),
                name: api_deployment_name(site),
            },
            min_replicas: Some(site.spec.api.hpa.min_replicas),
            max_replicas: site.spec.api.hpa.max_replicas,
            metrics: Some(vec![MetricSpec {
                type_: "Resource".to_string(),
                resource: Some(ResourceMetricSource {
                    name: "cpu".to_string(),
                    target: MetricTarget {
                        type_: "Utilization".to_string(),
                        average_utilization: Some(
                            site.spec.api.hpa.target_cpu_utilization_percentage,
                        ),
                        ..MetricTarget::default()
                    },
                }),
                ..MetricSpec::default()
            }]),
            ..HorizontalPodAutoscalerSpec::default()
        }),
        ..HorizontalPodAutoscaler::default()
    })
}

pub fn build_api_pdb(site: &PelagoSite) -> Option<PodDisruptionBudget> {
    if !site.spec.api.pdb.enabled {
        return None;
    }

    Some(PodDisruptionBudget {
        metadata: metadata(site, &api_pdb_name(site), labels(site), BTreeMap::new()),
        spec: Some(PodDisruptionBudgetSpec {
            min_available: Some(
                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(
                    site.spec.api.pdb.min_available,
                ),
            ),
            selector: Some(LabelSelector {
                match_labels: Some(api_selector_labels(site)),
                ..LabelSelector::default()
            }),
            ..PodDisruptionBudgetSpec::default()
        }),
        ..PodDisruptionBudget::default()
    })
}

fn metadata(
    site: &PelagoSite,
    name: &str,
    labels: BTreeMap<String, String>,
    annotations: BTreeMap<String, String>,
) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.to_string()),
        namespace: site.namespace(),
        labels: Some(labels),
        annotations: if annotations.is_empty() {
            None
        } else {
            Some(annotations)
        },
        owner_references: site.controller_owner_ref(&()).map(|owner| vec![owner]),
        ..ObjectMeta::default()
    }
}

fn secret_env_var(name: &str, secret_ref: &SecretKeyRef) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value_from: Some(EnvVarSource {
            secret_key_ref: Some(SecretKeySelector {
                key: secret_ref.key.clone(),
                name: secret_ref.name.clone(),
                optional: Some(false),
            }),
            ..EnvVarSource::default()
        }),
        ..EnvVar::default()
    }
}

fn secret_volume(volume_name: &str, secret_ref: &SecretKeyRef, path: &str) -> Volume {
    Volume {
        name: volume_name.to_string(),
        secret: Some(SecretVolumeSource {
            secret_name: Some(secret_ref.name.clone()),
            items: Some(vec![k8s_openapi::api::core::v1::KeyToPath {
                key: secret_ref.key.clone(),
                path: path.to_string(),
                ..k8s_openapi::api::core::v1::KeyToPath::default()
            }]),
            ..SecretVolumeSource::default()
        }),
        ..Volume::default()
    }
}

fn tcp_probe(port_name: &str, initial_delay_seconds: i32, period_seconds: i32) -> Probe {
    Probe {
        tcp_socket: Some(TCPSocketAction {
            port: k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::String(
                port_name.to_string(),
            ),
            host: None,
        }),
        initial_delay_seconds: Some(initial_delay_seconds),
        period_seconds: Some(period_seconds),
        ..Probe::default()
    }
}

fn peers_csv(peers: &[ReplicationPeerSpec]) -> String {
    peers
        .iter()
        .map(|peer| format!("{}={}", peer.site_id, peer.endpoint))
        .collect::<Vec<_>>()
        .join(",")
}

fn scopes_csv(scopes: &[ReplicationScopeSpec], default_db: &str, default_ns: &str) -> String {
    if scopes.is_empty() {
        return format!("{}/{}", default_db, default_ns);
    }

    scopes
        .iter()
        .map(|scope| format!("{}/{}", scope.database, scope.namespace))
        .collect::<Vec<_>>()
        .join(",")
}

fn shared_peer_api_key_secret(peers: &[ReplicationPeerSpec]) -> Option<SecretKeyRef> {
    peers
        .iter()
        .find_map(|peer| peer.api_key_secret_ref.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{FdbSpec, PelagoSiteSpec, SecretKeyRef, SiteSpec};

    fn sample_site() -> PelagoSite {
        PelagoSite::new(
            "sample",
            PelagoSiteSpec {
                site: SiteSpec {
                    id: 1,
                    name: "site-a".to_string(),
                },
                fdb: FdbSpec {
                    cluster_file_secret_ref: SecretKeyRef {
                        name: "pelagodb-secrets".to_string(),
                        key: "fdb.cluster".to_string(),
                    },
                },
                ..Default::default()
            },
        )
    }

    #[test]
    fn builds_expected_names() {
        let site = sample_site();
        assert_eq!(config_map_name(&site), "sample-config");
        assert_eq!(api_deployment_name(&site), "sample-api");
        assert_eq!(replicator_deployment_name(&site), "sample-replicator");
    }

    #[test]
    fn hpa_is_optional() {
        let mut site = sample_site();
        site.spec.api.hpa.enabled = false;
        assert!(build_api_hpa(&site).is_none());

        site.spec.api.hpa.enabled = true;
        assert!(build_api_hpa(&site).is_some());
    }
}
