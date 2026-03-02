# PelagoDB Kubernetes Deployment (Legacy Static Manifests)

This directory is retained for manual/static Kubernetes deployments.

Preferred path for new installs: use the operator + `PelagoSite` CR:
- `docs/operations/kubernetes-operator.md`
- `deploy/operator/chart/pelago-operator`

This folder provides a split deployment model:

- `api` pods: handle read/query/write traffic, cache/projector enabled, replication disabled.
- `replicator` pods: perform pull CDC replication only, lease-gated singleton behavior.

## Files

- `configmap.yaml`: shared non-secret runtime config.
- `secret.example.yaml`: template for sensitive values.
- `api-deployment.yaml`: horizontally scalable API tier.
- `api-service.yaml`: ClusterIP service for API tier.
- `api-hpa.yaml`: optional HPA for API pods.
- `api-pdb.yaml`: optional API PodDisruptionBudget.
- `replicator-deployment.yaml`: dedicated replicator tier.
- `CHECKLIST.md`: rollout and verification checklist.

## Quick Start

1. Create namespace and secrets:

```bash
kubectl create namespace pelagodb
kubectl apply -n pelagodb -f deploy/k8s/secret.example.yaml
```

2. Apply core manifests:

```bash
kubectl apply -n pelagodb -f deploy/k8s/configmap.yaml
kubectl apply -n pelagodb -f deploy/k8s/api-deployment.yaml
kubectl apply -n pelagodb -f deploy/k8s/api-service.yaml
kubectl apply -n pelagodb -f deploy/k8s/replicator-deployment.yaml
```

3. Optional autoscaling and disruption budget:

```bash
kubectl apply -n pelagodb -f deploy/k8s/api-hpa.yaml
kubectl apply -n pelagodb -f deploy/k8s/api-pdb.yaml
```

4. Follow `deploy/k8s/CHECKLIST.md` for validation and failover drills.
