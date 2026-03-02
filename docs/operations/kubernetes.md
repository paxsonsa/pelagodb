# Kubernetes Deployment

Recommended path: use the `pelago-operator` and `PelagoSite` custom resource.

## Recommended: Operator Workflow

Use the operator for day-1 and day-2 lifecycle management:

- declarative site topology (`api` + `replicator`) in one CR
- optional API HPA and API PDB management
- status/conditions on `PelagoSite`
- validating/mutating admission webhooks

See [Kubernetes Operator](kubernetes-operator.md) for installation and examples.

## Legacy: Static Manifests (`deploy/k8s`)

Static manifests are retained for compatibility but are legacy/manual path:

- `deploy/k8s/configmap.yaml`
- `deploy/k8s/api-deployment.yaml`
- `deploy/k8s/api-service.yaml`
- `deploy/k8s/api-hpa.yaml`
- `deploy/k8s/api-pdb.yaml`
- `deploy/k8s/replicator-deployment.yaml`

### Legacy-to-CR Field Mapping

| Legacy manifest field | `PelagoSite` field |
|---|---|
| `PELAGO_SITE_ID` / `PELAGO_SITE_NAME` | `spec.site.id` / `spec.site.name` |
| `PELAGO_FDB_CLUSTER` + secret mount | `spec.fdb.clusterFileSecretRef` |
| API deployment replicas | `spec.api.replicas` |
| API service type/port | `spec.api.service.serviceType` / `spec.api.service.port` |
| API HPA settings | `spec.api.hpa.*` |
| API PDB settings | `spec.api.pdb.*` |
| Replicator deployment replicas | `spec.replicator.replicas` |
| `PELAGO_REPLICATION_PEERS` | `spec.replicator.peers[]` |
| `PELAGO_REPLICATION_SCOPES` | `spec.replicator.scopes[]` |
| Replication lease settings | `spec.replicator.lease.*` |
| Replication TLS settings | `spec.replicator.tls.*` |

## Related

- [Kubernetes Operator](kubernetes-operator.md)
- [Deployment Guide](deployment.md)
- [Replication Operations](replication-operations.md)
- [Configuration Reference](../reference/configuration.md)
