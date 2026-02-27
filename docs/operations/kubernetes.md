# Kubernetes Deployment

Kubernetes manifests and guidance for running PelagoDB with split API/replicator topology.

## Architecture

The recommended K8s deployment splits PelagoDB into two deployment types:

| Deployment | Role | Key Settings |
|---|---|---|
| `pelago-api` | Serve client gRPC traffic | `PELAGO_REPLICATION_ENABLED=false`, `PELAGO_CACHE_ENABLED=true` |
| `pelago-replicator` | Pull CDC from remote sites | `PELAGO_REPLICATION_ENABLED=true` |

Both connect to the same FoundationDB cluster.

## Manifests

Reference manifests are in `deploy/k8s/`:

```
deploy/k8s/
├── README.md
├── CHECKLIST.md
├── api-deployment.yaml
├── replicator-deployment.yaml
├── service.yaml
├── configmap.yaml
└── hpa.yaml
```

## API Deployment

Key configuration:

```yaml
env:
  - name: PELAGO_FDB_CLUSTER
    value: "/etc/foundationdb/fdb.cluster"
  - name: PELAGO_SITE_ID
    value: "1"
  - name: PELAGO_SITE_NAME
    value: "k8s-site-1"
  - name: PELAGO_LISTEN_ADDR
    value: "0.0.0.0:27615"
  - name: PELAGO_REPLICATION_ENABLED
    value: "false"
  - name: PELAGO_CACHE_ENABLED
    value: "true"
  - name: PELAGO_AUTH_REQUIRED
    value: "true"
```

## Replicator Deployment

Key differences from API deployment:

```yaml
env:
  - name: PELAGO_REPLICATION_ENABLED
    value: "true"
  - name: PELAGO_REPLICATION_PEERS
    value: "2=pelago-site-2.remote:27616"
  - name: PELAGO_REPLICATION_LEASE_ENABLED
    value: "true"
```

The replicator uses lease-gated singleton behavior — only one replica actively pulls per scope. Additional replicas serve as standby for failover.

## Horizontal Pod Autoscaling

API nodes can scale horizontally based on CPU/request metrics:

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: pelago-api
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: pelago-api
  minReplicas: 2
  maxReplicas: 10
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
```

Replicator nodes should generally **not** be auto-scaled — lease gating ensures only one active replica per scope.

## Health Checks

Configure liveness and readiness probes against the gRPC health service:

```yaml
livenessProbe:
  exec:
    command:
      - grpcurl
      - -plaintext
      - -d
      - '{"service":"pelago"}'
      - localhost:27615
      - pelago.v1.HealthService/Check
  initialDelaySeconds: 10
  periodSeconds: 30
```

## Deployment Checklist

See `deploy/k8s/CHECKLIST.md` for the full pre-deployment checklist, including:

- [ ] FDB cluster accessible from K8s pods
- [ ] Site ID unique across all sites
- [ ] Auth configured with appropriate API keys/mTLS
- [ ] Replication peers resolvable from replicator pods
- [ ] Cache volume provisioned for API nodes
- [ ] Resource limits set for both deployments
- [ ] HPA configured for API deployment
- [ ] Monitoring endpoints exposed

## Related

- [Deployment Guide](deployment.md) — general deployment topologies
- [Configuration Reference](../reference/configuration.md) — all settings
- [Replication Operations](replication-operations.md) — monitoring replication
- [Production Checklist](production-checklist.md) — pre-production readiness
