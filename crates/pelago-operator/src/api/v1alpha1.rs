use k8s_openapi::api::core::v1::{
    Affinity, EnvVar, ResourceRequirements, Toleration, Volume, VolumeMount,
};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[kube(
    group = "pelago.pelagodb.io",
    version = "v1alpha1",
    kind = "PelagoSite",
    plural = "pelagosites",
    namespaced,
    shortname = "pgsite",
    status = "PelagoSiteStatus"
)]
pub struct PelagoSiteSpec {
    #[serde(default)]
    pub image: ImageSpec,
    pub site: SiteSpec,
    pub fdb: FdbSpec,
    #[serde(default)]
    pub defaults: DefaultsSpec,
    #[serde(default)]
    pub auth: AuthSpec,
    #[serde(default)]
    pub api: ApiTierSpec,
    #[serde(default)]
    pub replicator: ReplicatorSpec,
    #[serde(default)]
    pub pod_template: PodTemplateOverrides,
}

impl Default for PelagoSiteSpec {
    fn default() -> Self {
        Self {
            image: ImageSpec::default(),
            site: SiteSpec {
                id: 1,
                name: "default".to_string(),
            },
            fdb: FdbSpec {
                cluster_file_secret_ref: SecretKeyRef {
                    name: "pelagodb-secrets".to_string(),
                    key: "fdb.cluster".to_string(),
                },
            },
            defaults: DefaultsSpec::default(),
            auth: AuthSpec::default(),
            api: ApiTierSpec::default(),
            replicator: ReplicatorSpec::default(),
            pod_template: PodTemplateOverrides::default(),
        }
    }
}

impl PelagoSiteSpec {
    pub fn referenced_secret_keys(&self) -> Vec<SecretKeyRef> {
        let mut refs = Vec::new();
        refs.push(self.fdb.cluster_file_secret_ref.clone());

        if let Some(secret) = self.auth.api_keys_secret_ref.clone() {
            refs.push(secret);
        }

        for peer in &self.replicator.peers {
            if let Some(secret) = peer.api_key_secret_ref.clone() {
                refs.push(secret);
            }
        }

        if let Some(secret) = self.replicator.tls.ca_bundle_secret_ref.clone() {
            refs.push(secret);
        }
        if let Some(secret) = self.replicator.tls.client_cert_secret_ref.clone() {
            refs.push(secret);
        }
        if let Some(secret) = self.replicator.tls.client_key_secret_ref.clone() {
            refs.push(secret);
        }

        let mut dedupe = BTreeSet::new();
        refs.into_iter()
            .filter(|s| dedupe.insert((s.name.clone(), s.key.clone())))
            .collect()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImageSpec {
    pub repository: String,
    pub tag: String,
    pub pull_policy: String,
}

impl Default for ImageSpec {
    fn default() -> Self {
        Self {
            repository: "ghcr.io/pelagodb/pelago-server".to_string(),
            tag: "latest".to_string(),
            pull_policy: "IfNotPresent".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SiteSpec {
    pub id: u8,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    pub name: String,
    pub key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FdbSpec {
    pub cluster_file_secret_ref: SecretKeyRef,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DefaultsSpec {
    pub database: String,
    pub namespace: String,
}

impl Default for DefaultsSpec {
    fn default() -> Self {
        Self {
            database: "default".to_string(),
            namespace: "default".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthSpec {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub api_keys_secret_ref: Option<SecretKeyRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTierSpec {
    pub replicas: i32,
    #[serde(default)]
    pub service: ApiServiceSpec,
    #[serde(default)]
    pub resources: Option<ResourceRequirements>,
    #[serde(default)]
    pub cache: CacheSpec,
    #[serde(default)]
    pub hpa: ApiHpaSpec,
    #[serde(default)]
    pub pdb: ApiPdbSpec,
}

impl Default for ApiTierSpec {
    fn default() -> Self {
        Self {
            replicas: 3,
            service: ApiServiceSpec::default(),
            resources: None,
            cache: CacheSpec::default(),
            hpa: ApiHpaSpec::default(),
            pdb: ApiPdbSpec::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiServiceSpec {
    pub service_type: ServiceType,
    pub port: i32,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

impl Default for ApiServiceSpec {
    fn default() -> Self {
        Self {
            service_type: ServiceType::ClusterIP,
            port: 27615,
            annotations: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub enum ServiceType {
    #[default]
    ClusterIP,
    NodePort,
    LoadBalancer,
}

impl ServiceType {
    pub fn as_k8s_value(&self) -> &'static str {
        match self {
            Self::ClusterIP => "ClusterIP",
            Self::NodePort => "NodePort",
            Self::LoadBalancer => "LoadBalancer",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CacheSpec {
    pub enabled: bool,
    pub path: String,
    pub projector_batch_size: usize,
}

impl Default for CacheSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            path: "/var/lib/pelago/cache".to_string(),
            projector_batch_size: 1000,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiHpaSpec {
    pub enabled: bool,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub target_cpu_utilization_percentage: i32,
}

impl Default for ApiHpaSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            min_replicas: 3,
            max_replicas: 15,
            target_cpu_utilization_percentage: 65,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiPdbSpec {
    pub enabled: bool,
    pub min_available: i32,
}

impl Default for ApiPdbSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            min_available: 2,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplicatorSpec {
    #[serde(default)]
    pub enabled: Option<bool>,
    pub replicas: i32,
    #[serde(default)]
    pub resources: Option<ResourceRequirements>,
    pub batch_size: usize,
    pub poll_ms: u64,
    #[serde(default)]
    pub lease: ReplicatorLeaseSpec,
    #[serde(default)]
    pub scopes: Vec<ReplicationScopeSpec>,
    #[serde(default)]
    pub peers: Vec<ReplicationPeerSpec>,
    #[serde(default)]
    pub tls: ReplicationTlsSpec,
}

impl Default for ReplicatorSpec {
    fn default() -> Self {
        Self {
            enabled: None,
            replicas: 2,
            resources: None,
            batch_size: 512,
            poll_ms: 300,
            lease: ReplicatorLeaseSpec::default(),
            scopes: Vec::new(),
            peers: Vec::new(),
            tls: ReplicationTlsSpec::default(),
        }
    }
}

impl ReplicatorSpec {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(!self.peers.is_empty())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplicatorLeaseSpec {
    pub enabled: bool,
    pub ttl_ms: u64,
    pub heartbeat_ms: u64,
}

impl Default for ReplicatorLeaseSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_ms: 10_000,
            heartbeat_ms: 2_000,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationScopeSpec {
    pub database: String,
    pub namespace: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationPeerSpec {
    pub site_id: String,
    pub endpoint: String,
    #[serde(default)]
    pub api_key_secret_ref: Option<SecretKeyRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationTlsSpec {
    #[serde(default)]
    pub mode: ReplicationTlsMode,
    #[serde(default)]
    pub ca_bundle_secret_ref: Option<SecretKeyRef>,
    #[serde(default)]
    pub client_cert_secret_ref: Option<SecretKeyRef>,
    #[serde(default)]
    pub client_key_secret_ref: Option<SecretKeyRef>,
    #[serde(default)]
    pub server_name: Option<String>,
}

impl Default for ReplicationTlsSpec {
    fn default() -> Self {
        Self {
            mode: ReplicationTlsMode::Disabled,
            ca_bundle_secret_ref: None,
            client_cert_secret_ref: None,
            client_key_secret_ref: None,
            server_name: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
#[serde(rename_all = "PascalCase")]
pub enum ReplicationTlsMode {
    #[default]
    Disabled,
    SystemRoots,
    CustomCA,
    MutualTLS,
}

impl ReplicationTlsMode {
    pub fn as_env_value(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::SystemRoots => "system-roots",
            Self::CustomCA => "custom-ca",
            Self::MutualTLS => "mutual-tls",
        }
    }

    pub fn tls_required(&self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodTemplateOverrides {
    #[serde(default)]
    pub api: PodTemplateSpec,
    #[serde(default)]
    pub replicator: PodTemplateSpec,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PodTemplateSpec {
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
    #[serde(default)]
    pub node_selector: BTreeMap<String, String>,
    #[serde(default)]
    pub tolerations: Vec<Toleration>,
    #[serde(default)]
    pub affinity: Option<Affinity>,
    #[serde(default)]
    pub extra_env: Vec<EnvVar>,
    #[serde(default)]
    pub extra_volume_mounts: Vec<VolumeMount>,
    #[serde(default)]
    pub extra_volumes: Vec<Volume>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PelagoSiteStatus {
    #[serde(default)]
    pub observed_generation: Option<i64>,
    #[serde(default)]
    pub conditions: Vec<StatusCondition>,
    #[serde(default)]
    pub api: Option<ApiTierStatus>,
    #[serde(default)]
    pub replicator: Option<ReplicatorTierStatus>,
    #[serde(default)]
    pub last_reconcile_time: Option<String>,
    #[serde(default)]
    pub managed_resources: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusCondition {
    #[serde(rename = "type")]
    pub condition_type: String,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    pub last_transition_time: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiTierStatus {
    pub ready_replicas: i32,
    pub desired_replicas: i32,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub hpa_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplicatorTierStatus {
    pub ready_replicas: i32,
    pub desired_replicas: i32,
}
