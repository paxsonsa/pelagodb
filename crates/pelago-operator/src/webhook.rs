use crate::api::{PelagoSite, ReplicationTlsMode};
use axum::{extract::State, Json};
use kube::core::admission::{AdmissionRequest, AdmissionResponse, AdmissionReview};
use kube::core::DynamicObject;
use std::collections::HashSet;

#[derive(Clone, Default)]
pub struct WebhookState;

pub async fn mutate(
    State(_state): State<WebhookState>,
    Json(review): Json<AdmissionReview<DynamicObject>>,
) -> Json<AdmissionReview<DynamicObject>> {
    let request: AdmissionRequest<DynamicObject> = match review.try_into() {
        Ok(req) => req,
        Err(err) => {
            let response = AdmissionResponse::invalid(format!(
                "failed to deserialize mutating admission request: {}",
                err
            ));
            return Json(response.into_review());
        }
    };

    let mut response = AdmissionResponse::from(&request);
    if let Some(object) = request.object.clone() {
        let original = match serde_json::to_value(&object) {
            Ok(value) => value,
            Err(err) => {
                let denied = response.deny(format!(
                    "failed to serialize original object for defaulting: {}",
                    err
                ));
                return Json(denied.into_review());
            }
        };

        let mut site: PelagoSite = match serde_json::from_value(original.clone()) {
            Ok(site) => site,
            Err(err) => {
                let denied = response.deny(format!(
                    "failed to deserialize PelagoSite from request object: {}",
                    err
                ));
                return Json(denied.into_review());
            }
        };
        apply_defaults(&mut site);

        let updated = match serde_json::to_value(&site) {
            Ok(value) => value,
            Err(err) => {
                let denied = response.deny(format!(
                    "failed to serialize mutated object for defaulting: {}",
                    err
                ));
                return Json(denied.into_review());
            }
        };

        let patch = json_patch::diff(&original, &updated);
        if !patch.0.is_empty() {
            response = match response.with_patch(patch) {
                Ok(patched) => patched,
                Err(err) => AdmissionResponse::from(&request)
                    .deny(format!("failed to create JSON patch: {}", err)),
            };
        }
    }

    Json(response.into_review())
}

pub async fn validate(
    State(_state): State<WebhookState>,
    Json(review): Json<AdmissionReview<DynamicObject>>,
) -> Json<AdmissionReview<DynamicObject>> {
    let request: AdmissionRequest<DynamicObject> = match review.try_into() {
        Ok(req) => req,
        Err(err) => {
            let response = AdmissionResponse::invalid(format!(
                "failed to deserialize validating admission request: {}",
                err
            ));
            return Json(response.into_review());
        }
    };

    let mut response = AdmissionResponse::from(&request);
    if let Some(object) = request.object.clone() {
        let mut site: PelagoSite =
            match serde_json::from_value(match serde_json::to_value(object) {
                Ok(value) => value,
                Err(err) => {
                    let denied = response.deny(format!(
                        "failed to serialize validating object for decode: {}",
                        err
                    ));
                    return Json(denied.into_review());
                }
            }) {
                Ok(site) => site,
                Err(err) => {
                    let denied = response.deny(format!(
                        "failed to deserialize PelagoSite for validation: {}",
                        err
                    ));
                    return Json(denied.into_review());
                }
            };
        apply_defaults(&mut site);
        let errors = validate_site(&site);
        if !errors.is_empty() {
            response = response.deny(errors.join("; "));
        }
    }

    Json(response.into_review())
}

pub fn apply_defaults(site: &mut PelagoSite) {
    if site.spec.replicator.enabled.is_none() {
        site.spec.replicator.enabled = Some(!site.spec.replicator.peers.is_empty());
    }

    if site.spec.replicator.scopes.is_empty() {
        site.spec
            .replicator
            .scopes
            .push(crate::api::ReplicationScopeSpec {
                database: site.spec.defaults.database.clone(),
                namespace: site.spec.defaults.namespace.clone(),
            });
    }
}

pub fn validate_site(site: &PelagoSite) -> Vec<String> {
    let mut errors = Vec::new();

    if site.spec.fdb.cluster_file_secret_ref.name.trim().is_empty() {
        errors.push("spec.fdb.clusterFileSecretRef.name must not be empty".to_string());
    }
    if site.spec.fdb.cluster_file_secret_ref.key.trim().is_empty() {
        errors.push("spec.fdb.clusterFileSecretRef.key must not be empty".to_string());
    }

    let local_site = site.spec.site.id.to_string();
    let mut peer_ids = HashSet::new();
    for peer in &site.spec.replicator.peers {
        if peer.site_id.trim().is_empty() {
            errors.push("replicator peer siteId must not be empty".to_string());
        }
        if peer.endpoint.trim().is_empty() {
            errors.push("replicator peer endpoint must not be empty".to_string());
        }
        if peer.site_id == local_site {
            errors.push("replicator peer siteId must not equal local site.id".to_string());
        }
        if !peer_ids.insert(peer.site_id.clone()) {
            errors.push(format!(
                "duplicate replicator peer siteId '{}' in spec.replicator.peers",
                peer.site_id
            ));
        }
    }

    let mut scopes = HashSet::new();
    for scope in &site.spec.replicator.scopes {
        if scope.database.trim().is_empty() || scope.namespace.trim().is_empty() {
            errors.push("replicator scope database/namespace must both be non-empty".to_string());
        }
        if !scopes.insert((scope.database.clone(), scope.namespace.clone())) {
            errors.push(format!(
                "duplicate replication scope '{}/{}'",
                scope.database, scope.namespace
            ));
        }
    }

    if site.spec.api.hpa.min_replicas <= 0 {
        errors.push("spec.api.hpa.minReplicas must be > 0".to_string());
    }
    if site.spec.api.hpa.max_replicas <= 0 {
        errors.push("spec.api.hpa.maxReplicas must be > 0".to_string());
    }
    if site.spec.api.hpa.min_replicas > site.spec.api.hpa.max_replicas {
        errors.push("spec.api.hpa.minReplicas must be <= maxReplicas".to_string());
    }

    if site.spec.replicator.lease.heartbeat_ms == 0 || site.spec.replicator.lease.ttl_ms == 0 {
        errors.push("replicator lease heartbeatMs/ttlMs must be > 0".to_string());
    }
    if site.spec.replicator.lease.ttl_ms < site.spec.replicator.lease.heartbeat_ms.saturating_mul(2)
    {
        errors.push("replicator lease ttlMs must be at least 2x heartbeatMs".to_string());
    }

    validate_tls(site, &mut errors);
    validate_peer_api_key_constraints(site, &mut errors);

    errors
}

fn validate_peer_api_key_constraints(site: &PelagoSite, errors: &mut Vec<String>) {
    // Server currently supports a single replication API key; all peers must share the same reference.
    let refs: Vec<_> = site
        .spec
        .replicator
        .peers
        .iter()
        .filter_map(|peer| peer.api_key_secret_ref.as_ref())
        .map(|secret| (secret.name.as_str(), secret.key.as_str()))
        .collect();

    if refs.is_empty() {
        return;
    }

    let first = refs[0];
    if refs.iter().any(|candidate| *candidate != first) {
        errors.push(
            "all spec.replicator.peers[*].apiKeySecretRef values must match in v1".to_string(),
        );
    }
}

fn validate_tls(site: &PelagoSite, errors: &mut Vec<String>) {
    let tls = &site.spec.replicator.tls;
    match tls.mode {
        ReplicationTlsMode::Disabled => {
            if tls.ca_bundle_secret_ref.is_some()
                || tls.client_cert_secret_ref.is_some()
                || tls.client_key_secret_ref.is_some()
            {
                errors.push(
                    "replicator.tls secret refs require mode=SystemRoots|CustomCA|MutualTLS"
                        .to_string(),
                );
            }
        }
        ReplicationTlsMode::SystemRoots => {
            if tls.ca_bundle_secret_ref.is_some() {
                errors.push(
                    "replicator.tls.caBundleSecretRef is not used with mode=SystemRoots"
                        .to_string(),
                );
            }
            if tls.client_cert_secret_ref.is_some() || tls.client_key_secret_ref.is_some() {
                errors
                    .push("replicator.tls client cert/key refs require mode=MutualTLS".to_string());
            }
        }
        ReplicationTlsMode::CustomCA => {
            if tls.ca_bundle_secret_ref.is_none() {
                errors.push(
                    "replicator.tls.caBundleSecretRef is required with mode=CustomCA".to_string(),
                );
            }
            if tls.client_cert_secret_ref.is_some() || tls.client_key_secret_ref.is_some() {
                errors
                    .push("replicator.tls client cert/key refs require mode=MutualTLS".to_string());
            }
        }
        ReplicationTlsMode::MutualTLS => {
            if tls.ca_bundle_secret_ref.is_none()
                || tls.client_cert_secret_ref.is_none()
                || tls.client_key_secret_ref.is_none()
            {
                errors.push(
                    "replicator.tls mode=MutualTLS requires caBundleSecretRef, clientCertSecretRef, and clientKeySecretRef"
                        .to_string(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{PelagoSiteSpec, SecretKeyRef, SiteSpec};

    fn base_site() -> PelagoSite {
        PelagoSite::new(
            "sample",
            PelagoSiteSpec {
                site: SiteSpec {
                    id: 1,
                    name: "site-a".to_string(),
                },
                fdb: crate::api::FdbSpec {
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
    fn defaults_enable_replicator_if_peers_exist() {
        let mut site = base_site();
        site.spec
            .replicator
            .peers
            .push(crate::api::ReplicationPeerSpec {
                site_id: "2".to_string(),
                endpoint: "peer:27615".to_string(),
                api_key_secret_ref: None,
            });

        assert_eq!(site.spec.replicator.enabled, None);
        apply_defaults(&mut site);
        assert_eq!(site.spec.replicator.enabled, Some(true));
    }

    #[test]
    fn validation_rejects_duplicate_peer_site_ids() {
        let mut site = base_site();
        site.spec.replicator.peers = vec![
            crate::api::ReplicationPeerSpec {
                site_id: "2".to_string(),
                endpoint: "peer1:27615".to_string(),
                api_key_secret_ref: None,
            },
            crate::api::ReplicationPeerSpec {
                site_id: "2".to_string(),
                endpoint: "peer2:27615".to_string(),
                api_key_secret_ref: None,
            },
        ];

        let errors = validate_site(&site);
        assert!(errors
            .iter()
            .any(|msg| msg.contains("duplicate replicator peer")));
    }

    #[test]
    fn validation_rejects_custom_ca_without_ref() {
        let mut site = base_site();
        site.spec.replicator.tls.mode = ReplicationTlsMode::CustomCA;
        let errors = validate_site(&site);
        assert!(errors
            .iter()
            .any(|msg| msg.contains("caBundleSecretRef is required")));
    }
}
