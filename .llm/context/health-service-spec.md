# HealthService Specification

**Status:** Proposed addition to PelagoDB Specification v1.0
**Section:** 9.x gRPC API - HealthService

---

## Overview

HealthService provides health checking endpoints for container orchestration platforms (Kubernetes, Docker Swarm, Nomad). The design follows the [gRPC Health Checking Protocol](https://github.com/grpc/grpc/blob/master/doc/health-checking.md) standard with PelagoDB-specific extensions for component-level diagnostics.

## Recommendation: Use Standard gRPC Health Checking Protocol

**Recommended approach:** Implement the standard `grpc.health.v1.Health` service alongside a PelagoDB-specific `DiagnosticsService` for detailed component checks.

**Rationale:**
1. **Kubernetes native support:** `grpc_health_probe` and Kubernetes gRPC health checks work out-of-the-box with `grpc.health.v1`
2. **Client compatibility:** All gRPC client libraries include health checking interceptors
3. **Separation of concerns:** Standard health for orchestration, custom diagnostics for observability

---

## 9.x HealthService

### 9.x.1 Service Overview

| Service | Purpose | RPCs |
|---------|---------|------|
| `grpc.health.v1.Health` | Standard gRPC health checking | `Check`, `Watch` |
| `DiagnosticsService` | Detailed component health | `GetComponentHealth`, `GetReadiness` |

### 9.x.2 Liveness vs Readiness Semantics

| Probe Type | gRPC Method | Purpose | Failure Action |
|------------|-------------|---------|----------------|
| **Liveness** | `Health.Check("")` | Process is alive and not deadlocked | Restart container |
| **Readiness** | `Health.Check("pelago.v1.NodeService")` | Can serve traffic | Remove from load balancer |

**Liveness Check:**
- Verifies the gRPC server is responsive
- Does NOT check FDB connectivity (FDB outage should not trigger restarts)
- Must respond within 1 second
- Returns `SERVING` if the server event loop is running

**Readiness Check:**
- Verifies FDB connectivity with a lightweight probe
- Verifies schema cache is initialized (if applicable)
- Returns `SERVING` only when all dependencies are healthy
- Returns `NOT_SERVING` during startup initialization or dependency failures

### 9.x.3 Components Checked

| Component | Check Method | Liveness | Readiness |
|-----------|--------------|----------|-----------|
| gRPC server | Event loop heartbeat | Yes | Yes |
| FDB connection | `db.read_system_key()` | No | Yes |
| Schema cache | Cache initialized flag | No | Yes |
| Worker threads | Tokio runtime metrics | No | Optional |

### 9.x.4 Standard Health Proto (grpc.health.v1)

```protobuf
// Standard gRPC health checking protocol
// See: https://github.com/grpc/grpc/blob/master/doc/health-checking.md
syntax = "proto3";

package grpc.health.v1;

service Health {
  // Unary health check
  // service="" checks overall server health (liveness)
  // service="pelago.v1.NodeService" checks specific service (readiness)
  rpc Check(HealthCheckRequest) returns (HealthCheckResponse);

  // Streaming health watch (for long-poll health monitoring)
  rpc Watch(HealthCheckRequest) returns (stream HealthCheckResponse);
}

message HealthCheckRequest {
  // Service name to check. Empty string = overall server health.
  // Use fully qualified service name: "pelago.v1.NodeService"
  string service = 1;
}

message HealthCheckResponse {
  enum ServingStatus {
    UNKNOWN = 0;
    SERVING = 1;
    NOT_SERVING = 2;
    SERVICE_UNKNOWN = 3;  // Service not registered
  }
  ServingStatus status = 1;
}
```

### 9.x.5 DiagnosticsService Proto (PelagoDB-specific)

```protobuf
// PelagoDB-specific diagnostics for observability
syntax = "proto3";

package pelago.v1;

// ═══════════════════════════════════════════════════════════════════════════
// DIAGNOSTICS SERVICE
// ═══════════════════════════════════════════════════════════════════════════

service DiagnosticsService {
  // Get detailed health status of all components
  rpc GetComponentHealth(GetComponentHealthRequest) returns (GetComponentHealthResponse);

  // Get readiness status with initialization details
  rpc GetReadiness(GetReadinessRequest) returns (GetReadinessResponse);
}

// ═══════════════════════════════════════════════════════════════════════════
// COMPONENT HEALTH
// ═══════════════════════════════════════════════════════════════════════════

message GetComponentHealthRequest {
  // Optional: specific components to check. Empty = all components.
  repeated string components = 1;
}

message GetComponentHealthResponse {
  // Overall health status
  HealthStatus status = 1;

  // Individual component health
  map<string, ComponentHealth> components = 2;

  // Server metadata
  ServerInfo server = 3;
}

message ComponentHealth {
  // Component status
  HealthStatus status = 1;

  // Human-readable status message
  string message = 2;

  // Last successful check timestamp (Unix microseconds)
  int64 last_check_at = 3;

  // Check latency in microseconds
  int64 latency_us = 4;

  // Component-specific metadata
  map<string, string> metadata = 5;
}

enum HealthStatus {
  HEALTH_STATUS_UNSPECIFIED = 0;
  HEALTH_STATUS_HEALTHY = 1;
  HEALTH_STATUS_DEGRADED = 2;    // Operational but impaired
  HEALTH_STATUS_UNHEALTHY = 3;
}

message ServerInfo {
  // Server version
  string version = 1;

  // Site ID (u8 as string)
  string site_id = 2;

  // Server start time (Unix microseconds)
  int64 started_at = 3;

  // Current server time (Unix microseconds)
  int64 current_time = 4;

  // Uptime in seconds
  int64 uptime_seconds = 5;
}

// ═══════════════════════════════════════════════════════════════════════════
// READINESS
// ═══════════════════════════════════════════════════════════════════════════

message GetReadinessRequest {
  // Include detailed initialization steps
  bool include_details = 1;
}

message GetReadinessResponse {
  // Overall readiness
  bool ready = 1;

  // Reason if not ready
  string reason = 2;

  // Initialization steps (if include_details=true)
  repeated InitStep init_steps = 3;
}

message InitStep {
  // Step name (e.g., "fdb_connect", "schema_cache_init")
  string name = 1;

  // Step status
  InitStepStatus status = 2;

  // Duration in microseconds (0 if not started)
  int64 duration_us = 3;

  // Error message if failed
  string error = 4;
}

enum InitStepStatus {
  INIT_STEP_STATUS_UNSPECIFIED = 0;
  INIT_STEP_STATUS_PENDING = 1;
  INIT_STEP_STATUS_IN_PROGRESS = 2;
  INIT_STEP_STATUS_COMPLETE = 3;
  INIT_STEP_STATUS_FAILED = 4;
}
```

### 9.x.6 Kubernetes Configuration

```yaml
# Example Kubernetes deployment configuration
apiVersion: apps/v1
kind: Deployment
metadata:
  name: pelagodb
spec:
  template:
    spec:
      containers:
        - name: pelagodb
          image: pelagodb:latest
          ports:
            - containerPort: 27615
              name: grpc
          livenessProbe:
            grpc:
              port: 27615
              service: ""  # Empty = overall server health
            initialDelaySeconds: 5
            periodSeconds: 10
            timeoutSeconds: 1
            failureThreshold: 3
          readinessProbe:
            grpc:
              port: 27615
              service: "pelago.v1.NodeService"
            initialDelaySeconds: 10
            periodSeconds: 5
            timeoutSeconds: 2
            failureThreshold: 3
          startupProbe:
            grpc:
              port: 27615
              service: "pelago.v1.NodeService"
            initialDelaySeconds: 0
            periodSeconds: 2
            timeoutSeconds: 2
            failureThreshold: 30  # Allow 60s for FDB connection
```

### 9.x.7 Implementation Notes

#### Service Registration

Register all PelagoDB services with the health server at startup:

```rust
// pelago-api/src/health.rs
use tonic_health::server::{HealthReporter, HealthServer};

pub async fn create_health_service() -> (HealthReporter, HealthServer) {
    let (mut reporter, service) = tonic_health::server::health_reporter();

    // Register all services as NOT_SERVING initially
    let services = [
        "", // Overall server health (liveness)
        "pelago.v1.SchemaService",
        "pelago.v1.NodeService",
        "pelago.v1.EdgeService",
        "pelago.v1.QueryService",
        "pelago.v1.AdminService",
        "pelago.v1.DiagnosticsService",
    ];

    for svc in services {
        reporter.set_not_serving(svc).await;
    }

    (reporter, service)
}
```

#### FDB Health Check

Lightweight FDB connectivity check using a system key read:

```rust
// pelago-storage/src/health.rs
use fdb::Database;

/// Check FDB connectivity with minimal overhead.
/// Reads a system key that exists in every FDB cluster.
pub async fn check_fdb_health(db: &Database) -> Result<Duration, FdbError> {
    let start = Instant::now();

    db.run(|trx, _| async move {
        // Read "\xff/status/json" prefix existence
        // This is a lightweight operation that verifies connectivity
        let _ = trx.get(b"\xff\x02/coordinators", false).await?;
        Ok(())
    }).await?;

    Ok(start.elapsed())
}
```

#### Health Reporter Background Task

Continuous health monitoring with automatic status updates:

```rust
// pelago-server/src/health_monitor.rs
use std::time::Duration;
use tokio::time::interval;
use tonic_health::server::HealthReporter;

pub async fn run_health_monitor(
    reporter: HealthReporter,
    db: Database,
    schema_cache: Arc<SchemaCache>,
) {
    let mut interval = interval(Duration::from_secs(5));

    loop {
        interval.tick().await;

        // Check FDB
        let fdb_healthy = check_fdb_health(&db).await.is_ok();

        // Check schema cache
        let cache_ready = schema_cache.is_initialized();

        // Update service statuses
        let status = if fdb_healthy && cache_ready {
            ServingStatus::Serving
        } else {
            ServingStatus::NotServing
        };

        // Update all data services
        for svc in ["pelago.v1.NodeService", "pelago.v1.EdgeService", ...] {
            reporter.set_service_status(svc, status).await;
        }

        // Liveness always SERVING if we reach this point
        reporter.set_serving("").await;
    }
}
```

#### Startup Sequence

```rust
// pelago-server/src/main.rs
async fn main() -> Result<()> {
    // 1. Create health service (services start as NOT_SERVING)
    let (health_reporter, health_service) = create_health_service().await;

    // 2. Start gRPC server with health service
    let server = Server::builder()
        .add_service(health_service)
        .add_service(/* ... other services */)
        .serve(addr);

    // 3. Initialize dependencies (FDB, schema cache)
    let db = init_fdb(&config).await?;
    let schema_cache = init_schema_cache(&db).await?;

    // 4. Mark services as SERVING
    health_reporter.set_serving("").await;
    health_reporter.set_serving("pelago.v1.NodeService").await;
    // ... etc

    // 5. Start health monitor background task
    tokio::spawn(run_health_monitor(health_reporter, db, schema_cache));

    server.await?;
    Ok(())
}
```

### 9.x.8 Wire Examples

#### Liveness Check (grpcurl)

```bash
# Liveness probe - check overall server health
$ grpcurl -plaintext localhost:27615 grpc.health.v1.Health/Check
{
  "status": "SERVING"
}
```

#### Readiness Check (grpcurl)

```bash
# Readiness probe - check specific service
$ grpcurl -plaintext -d '{"service": "pelago.v1.NodeService"}' \
    localhost:27615 grpc.health.v1.Health/Check
{
  "status": "SERVING"
}
```

#### Component Health (grpcurl)

```bash
# Detailed component health
$ grpcurl -plaintext localhost:27615 pelago.v1.DiagnosticsService/GetComponentHealth
{
  "status": "HEALTH_STATUS_HEALTHY",
  "components": {
    "fdb": {
      "status": "HEALTH_STATUS_HEALTHY",
      "message": "Connected to cluster",
      "lastCheckAt": "1707912345000000",
      "latencyUs": "1234",
      "metadata": {
        "cluster_file": "/etc/foundationdb/fdb.cluster",
        "fdb_version": "7.1.25"
      }
    },
    "schema_cache": {
      "status": "HEALTH_STATUS_HEALTHY",
      "message": "Cache initialized with 12 schemas",
      "lastCheckAt": "1707912345000000",
      "metadata": {
        "schema_count": "12",
        "last_refresh": "1707912300000000"
      }
    }
  },
  "server": {
    "version": "1.0.0",
    "siteId": "1",
    "startedAt": "1707900000000000",
    "currentTime": "1707912345000000",
    "uptimeSeconds": "12345"
  }
}
```

---

## Crate Dependencies

Add to `pelago-api/Cargo.toml`:

```toml
[dependencies]
tonic-health = "0.11"  # Standard gRPC health checking
```

---

## File Locations

| File | Purpose |
|------|---------|
| `proto/pelago.proto` | Add DiagnosticsService definitions |
| `crates/pelago-api/src/health.rs` | Health service setup |
| `crates/pelago-api/src/diagnostics_service.rs` | DiagnosticsService implementation |
| `crates/pelago-storage/src/health.rs` | FDB health check |
| `crates/pelago-server/src/health_monitor.rs` | Background health monitoring |

---

## Summary

| Aspect | Standard (grpc.health.v1) | Extended (DiagnosticsService) |
|--------|---------------------------|-------------------------------|
| **Use case** | K8s probes, load balancers | Observability, debugging |
| **Response time** | <1ms | <10ms |
| **FDB check** | No (liveness), Yes (readiness) | Yes, with latency |
| **Schema cache check** | Yes (readiness) | Yes, with details |
| **Compatibility** | Universal gRPC | PelagoDB-specific |
