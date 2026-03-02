# pelago-operator

Rust Kubernetes operator for PelagoDB.

Responsibilities:

- reconciles `PelagoSite` CRs into API/replicator Deployments and related resources
- exposes mutating/validating admission webhooks for `PelagoSite`
- manages status conditions and observed readiness

Generate CRD YAML:

```bash
cargo run -p pelago-operator --bin pelago-operator-crdgen
```
