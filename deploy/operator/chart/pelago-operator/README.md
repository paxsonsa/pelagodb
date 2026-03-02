# pelago-operator Helm Chart

Install:

```bash
helm upgrade --install pelago-operator deploy/operator/chart/pelago-operator \
  --namespace pelagodb-system \
  --create-namespace
```

Apply a `PelagoSite` custom resource:

```bash
kubectl apply -n pelagodb -f deploy/operator/examples/single-site-basic.yaml
```
