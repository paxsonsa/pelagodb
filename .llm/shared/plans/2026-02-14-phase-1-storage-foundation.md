# PelagoDB Phase 1: Storage Foundation Implementation Plan

## Overview

This plan implements **Phase 1** of PelagoDB v1, establishing the core storage primitives, schema system, CRUD operations, query capabilities, and gRPC API that all subsequent phases depend on.

**Reference Documents:**
- Spec: `.llm/context/pelagodb-spec-v1.md` (sections 1-9, 13-16)
- Phase Overview: `.llm/shared/research/2026-02-14-pelagodb-implementation-phases.md`
- Archived Plan: `.llm/context/archive/phase-1-plan.md`

**Worktree:**
- Branch: `phase-1-storage-foundation`
- Path: `claude/worktrees/phase-1-storage-foundation`

## Current State Analysis

This is a **greenfield project**. The repository contains only:
- `.llm/` - Specifications and research documents
- `.git/` - Git repository
- `.claude/` - Claude configuration

No Rust code, Cargo.toml, proto files, or any implementation exists.

## Desired End State

After Phase 1 completion, PelagoDB will have:

1. **Working gRPC API** exposing:
   - SchemaService: RegisterSchema, GetSchema, ListSchemas
   - NodeService: CreateNode, GetNode, UpdateNode, DeleteNode
   - EdgeService: CreateEdge, DeleteEdge, ListEdges (streaming)
   - QueryService: FindNodes (streaming), Traverse (streaming), Explain
   - AdminService: DropIndex, StripProperty, GetJobStatus, DropEntityType, DropNamespace
   - HealthService: Check

2. **FDB-backed storage** with:
   - Schema registry with versioning
   - Node CRUD with index maintenance
   - Edge CRUD with bidirectional pairs
   - CDC entry emission (entries written but not consumed until Phase 2)
   - Job storage and status API (execution deferred to Phase 2)

3. **Query system** with:
   - CEL expression parsing and type-checking
   - Single-predicate index selection
   - Multi-hop traversal with per-hop filtering
   - Snapshot isolation for multi-batch queries

4. **Validation:**
   - All gRPC services respond correctly
   - Point lookup < 1ms p99
   - Index query < 10ms p99
   - Traversal (depth 4, 100 nodes) < 100ms p99

### Key Discoveries from Spec Analysis

1. **Node ID Format**: 9 bytes `[site_id:u8][counter:u64]` - compact, site-extractable, no cross-site coordination
2. **Keyspace Layout**: `(db, ns, subspace, ...)` hierarchy with `_sys`, `_db`, `_ns`, `_schema`, `_cdc`, `_jobs`, `_ids`, `data`, `loc`, `idx`, `edge` subspaces
3. **Edge Storage**: Paired entries (forward `f`, forward-metadata `fm`, reverse `r`) with optional sort key in key position
4. **CDC**: Versionstamp-keyed entries for zero-contention ordering
5. **Index Types**: Unique (point lookup), Equality (prefix scan), Range (sort-order encoded)

## What We're NOT Doing

**Explicitly out of scope for Phase 1:**

- **CDC Consumer Framework** - Phase 2 (M7-M8)
- **Background Job Execution** - Phase 2 (M9) - we store jobs but don't execute them
- **PQL Query Language** - Phase 3 (M10-M11)
- **RocksDB Cache Layer** - Phase 3 (M12-M13)
- **CLI Tool** - Phase 3 (M13.5-M13.7)
- **Watch/Subscriptions** - Phase 4 (M14-M17)
- **Multi-Site Replication** - Phase 5 (M18-M21)
- **Security/Authorization** - Phase 6 (M22-M24)

---

## Implementation Approach

Phase 1 is divided into **7 milestones** that build incrementally:

```
M0 (Project Scaffolding)
 │
 └── M1 (Schema Registry)
      │
      ├── M2 (Node Operations)
      │    │
      │    └── M3 (Edge Operations)
      │         │
      │         └── M5 (gRPC API) ──→ M6 (Testing)
      │              ▲
      └── M4 (Query System) ───┘
           (can start parallel with M3)
```

Each milestone ends with working, testable code.

---

## Phase 1.0: Project Scaffolding (M0)

### Overview
Create the Rust workspace, define core types, establish FDB connection, and set up the encoding layer.

### Changes Required:

#### 1. Workspace Root

**File**: `Cargo.toml`
```toml
[workspace]
resolver = "2"
members = [
    "crates/pelago-core",
    "crates/pelago-proto",
    "crates/pelago-storage",
    "crates/pelago-query",
    "crates/pelago-api",
    "crates/pelago-server",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
authors = ["PelagoDB Team"]
license = "Apache-2.0"
repository = "https://github.com/pelagodb/pelagodb"

[workspace.dependencies]
# Async
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
ciborium = "0.2"

# FDB
fdb = "0.5"

# gRPC
tonic = "0.12"
prost = "0.13"
prost-types = "0.13"

# Error handling
thiserror = "2"
anyhow = "1"

# Observability
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# CLI
clap = { version = "4", features = ["derive", "env"] }

# CEL
cel-interpreter = "0.9"

# Utilities
uuid = { version = "1", features = ["v7"] }
bytes = "1"
```

**File**: `Dockerfile`
```dockerfile
# Multi-stage build for PelagoDB server
FROM rust:1.82 AS builder

# Install FDB client libraries
RUN curl -L https://github.com/apple/foundationdb/releases/download/7.3.27/foundationdb-clients_7.3.27-1_amd64.deb -o fdb.deb \
    && dpkg -i fdb.deb \
    && rm fdb.deb

WORKDIR /app
COPY . .
RUN cargo build --release --bin pelago-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/lib/libfdb_c.so /usr/lib/
COPY --from=builder /app/target/release/pelago-server /usr/local/bin/
EXPOSE 50051
CMD ["pelago-server"]
```

**File**: `docker-compose.yml`
```yaml
version: '3.8'

services:
  fdb:
    image: foundationdb/foundationdb:7.3.27
    environment:
      FDB_NETWORKING_MODE: container
    volumes:
      - fdb-data:/var/fdb/data

  pelago:
    build: .
    depends_on:
      - fdb
    environment:
      PELAGO_FDB_CLUSTER: fdb:4500
      PELAGO_SITE_ID: 1
      PELAGO_LISTEN_ADDR: "[::]:50051"
      RUST_LOG: info
    ports:
      - "50051:50051"

volumes:
  fdb-data:
```

#### 2. Core Types Crate

**File**: `crates/pelago-core/Cargo.toml`
```toml
[package]
name = "pelago-core"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
ciborium.workspace = true
thiserror.workspace = true
bytes.workspace = true
```

**File**: `crates/pelago-core/src/lib.rs`
```rust
pub mod types;
pub mod encoding;
pub mod schema;
pub mod errors;
pub mod config;

pub use types::*;
pub use errors::*;
```

**File**: `crates/pelago-core/src/types.rs`
```rust
//! Core PelagoDB types: NodeId, EdgeId, Value, PropertyType

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Node ID: 9 bytes encoding site + sequence
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId {
    pub site: u8,
    pub seq: u64,
}

impl NodeId {
    pub fn new(site: u8, seq: u64) -> Self {
        Self { site, seq }
    }

    /// Serialize to 9 bytes for FDB key
    pub fn to_bytes(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = self.site;
        buf[1..9].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    /// Deserialize from 9 bytes
    pub fn from_bytes(bytes: &[u8; 9]) -> Self {
        Self {
            site: bytes[0],
            seq: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        }
    }

    /// Extract originating site
    pub fn origin_site(&self) -> u8 {
        self.site
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.site, self.seq)
    }
}

impl FromStr for NodeId {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('_').collect();
        if parts.len() != 2 {
            return Err(ParseIdError::InvalidFormat);
        }
        Ok(Self {
            site: parts[0].parse().map_err(|_| ParseIdError::InvalidSite)?,
            seq: parts[1].parse().map_err(|_| ParseIdError::InvalidSeq)?,
        })
    }
}

/// Edge ID: same 9-byte format as NodeId
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId {
    pub site: u8,
    pub seq: u64,
}

impl EdgeId {
    pub fn new(site: u8, seq: u64) -> Self {
        Self { site, seq }
    }

    pub fn to_bytes(&self) -> [u8; 9] {
        let mut buf = [0u8; 9];
        buf[0] = self.site;
        buf[1..9].copy_from_slice(&self.seq.to_be_bytes());
        buf
    }

    pub fn from_bytes(bytes: &[u8; 9]) -> Self {
        Self {
            site: bytes[0],
            seq: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        }
    }
}

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", self.site, self.seq)
    }
}

impl FromStr for EdgeId {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('_').collect();
        if parts.len() != 2 {
            return Err(ParseIdError::InvalidFormat);
        }
        Ok(Self {
            site: parts[0].parse().map_err(|_| ParseIdError::InvalidSite)?,
            seq: parts[1].parse().map_err(|_| ParseIdError::InvalidSeq)?,
        })
    }
}

/// Property value enum matching spec §4.3
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Timestamp(i64),
    Bytes(Vec<u8>),
    Null,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::String(_) => "string",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Timestamp(_) => "timestamp",
            Value::Bytes(_) => "bytes",
            Value::Null => "null",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// Property type enum for schema definitions
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PropertyType {
    String,
    Int,
    Float,
    Bool,
    Timestamp,
    Bytes,
}

impl PropertyType {
    pub fn matches(&self, value: &Value) -> bool {
        matches!(
            (self, value),
            (PropertyType::String, Value::String(_))
                | (PropertyType::Int, Value::Int(_))
                | (PropertyType::Float, Value::Float(_))
                | (PropertyType::Bool, Value::Bool(_))
                | (PropertyType::Timestamp, Value::Timestamp(_))
                | (PropertyType::Bytes, Value::Bytes(_))
                | (_, Value::Null)  // Null matches any type
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseIdError {
    #[error("invalid ID format, expected 'site_seq'")]
    InvalidFormat,
    #[error("invalid site ID")]
    InvalidSite,
    #[error("invalid sequence number")]
    InvalidSeq,
}
```

**File**: `crates/pelago-core/src/encoding.rs`
```rust
//! Encoding utilities: tuple encoding, CBOR helpers, sort-order encoding

use crate::{PelagoError, Value};
use bytes::{BufMut, Bytes, BytesMut};
use std::collections::HashMap;

/// Encode i64 for range index (sort-order preserving)
/// Flips sign bit for correct signed integer ordering
pub fn encode_int_for_index(n: i64) -> [u8; 8] {
    let mut bytes = n.to_be_bytes();
    bytes[0] ^= 0x80; // Flip sign bit
    bytes
}

/// Decode i64 from range index encoding
pub fn decode_int_from_index(bytes: &[u8; 8]) -> i64 {
    let mut buf = *bytes;
    buf[0] ^= 0x80;
    i64::from_be_bytes(buf)
}

/// Encode f64 for range index (IEEE 754 order-preserving)
pub fn encode_float_for_index(f: f64) -> [u8; 8] {
    let bits = f.to_bits();
    let encoded = if bits & 0x8000_0000_0000_0000 != 0 {
        !bits // Negative: invert all bits
    } else {
        bits ^ 0x8000_0000_0000_0000 // Positive: flip sign bit
    };
    encoded.to_be_bytes()
}

/// Decode f64 from range index encoding
pub fn decode_float_from_index(bytes: &[u8; 8]) -> f64 {
    let encoded = u64::from_be_bytes(*bytes);
    let bits = if encoded & 0x8000_0000_0000_0000 == 0 {
        !encoded // Was negative
    } else {
        encoded ^ 0x8000_0000_0000_0000 // Was positive
    };
    f64::from_bits(bits)
}

/// Encode a Value for index key position
pub fn encode_value_for_index(value: &Value) -> Result<Vec<u8>, PelagoError> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Int(n) => Ok(encode_int_for_index(*n).to_vec()),
        Value::Float(f) => Ok(encode_float_for_index(*f).to_vec()),
        Value::Timestamp(t) => Ok(encode_int_for_index(*t).to_vec()),
        Value::Bool(b) => Ok(vec![if *b { 0x01 } else { 0x00 }]),
        Value::Bytes(b) => Ok(b.clone()),
        Value::Null => Err(PelagoError::Internal("Cannot encode null for index".into())),
    }
}

/// Serialize properties to CBOR
pub fn encode_cbor<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, PelagoError> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf)
        .map_err(|e| PelagoError::Internal(format!("CBOR encode error: {}", e)))?;
    Ok(buf)
}

/// Deserialize from CBOR
pub fn decode_cbor<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, PelagoError> {
    ciborium::from_reader(bytes)
        .map_err(|e| PelagoError::Internal(format!("CBOR decode error: {}", e)))
}

/// Build a tuple-encoded key from components
/// Uses FDB tuple layer conventions
pub struct TupleBuilder {
    buf: BytesMut,
}

impl TupleBuilder {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
        }
    }

    /// Add string element
    pub fn add_string(mut self, s: &str) -> Self {
        // Type byte for string + null-terminated
        self.buf.put_u8(0x02);
        self.buf.put_slice(s.as_bytes());
        self.buf.put_u8(0x00);
        self
    }

    /// Add bytes element
    pub fn add_bytes(mut self, b: &[u8]) -> Self {
        self.buf.put_u8(0x01);
        self.buf.put_slice(b);
        self.buf.put_u8(0x00);
        self
    }

    /// Add i64 element (big-endian with sign flip for ordering)
    pub fn add_int(mut self, n: i64) -> Self {
        self.buf.put_u8(0x14);
        self.buf.put_slice(&encode_int_for_index(n));
        self
    }

    /// Build the final key bytes
    pub fn build(self) -> Bytes {
        self.buf.freeze()
    }

    /// Build with a range-end suffix for prefix scans
    pub fn build_range_end(mut self) -> Bytes {
        self.buf.put_u8(0xFF);
        self.buf.freeze()
    }
}

impl Default for TupleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int_encoding_preserves_order() {
        let values = vec![-1000i64, -1, 0, 1, 1000];
        let encoded: Vec<_> = values.iter().map(|&v| encode_int_for_index(v)).collect();

        for i in 0..encoded.len() - 1 {
            assert!(encoded[i] < encoded[i + 1], "Order not preserved");
        }
    }

    #[test]
    fn test_int_roundtrip() {
        for n in [-1000i64, -1, 0, 1, 1000, i64::MIN, i64::MAX] {
            let encoded = encode_int_for_index(n);
            let decoded = decode_int_from_index(&encoded);
            assert_eq!(n, decoded);
        }
    }

    #[test]
    fn test_float_encoding_preserves_order() {
        let values = vec![-100.0f64, -0.1, 0.0, 0.1, 100.0];
        let encoded: Vec<_> = values.iter().map(|&v| encode_float_for_index(v)).collect();

        for i in 0..encoded.len() - 1 {
            assert!(encoded[i] < encoded[i + 1], "Order not preserved");
        }
    }

    #[test]
    fn test_cbor_roundtrip() {
        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::String("Alice".to_string()));
        props.insert("age".to_string(), Value::Int(30));

        let encoded = encode_cbor(&props).unwrap();
        let decoded: HashMap<String, Value> = decode_cbor(&encoded).unwrap();

        assert_eq!(props, decoded);
    }
}
```

**File**: `crates/pelago-core/src/errors.rs`
```rust
//! PelagoDB error types with gRPC status mapping

use std::collections::HashMap;

/// Main error type for PelagoDB
#[derive(Debug, thiserror::Error)]
pub enum PelagoError {
    // Validation errors
    #[error("Entity type '{entity_type}' not registered")]
    UnregisteredType { entity_type: String },

    #[error("Required property '{field}' missing for type '{entity_type}'")]
    MissingRequired { entity_type: String, field: String },

    #[error("Type mismatch for field '{field}': expected {expected}, got {actual}")]
    TypeMismatch {
        field: String,
        expected: String,
        actual: String,
    },

    #[error("Extra property '{field}' not allowed (extras_policy=reject)")]
    ExtraProperty { field: String },

    #[error("CEL syntax error in '{expression}': {message}")]
    CelSyntax { expression: String, message: String },

    #[error("CEL type error for field '{field}': {message}")]
    CelType { field: String, message: String },

    #[error("Invalid ID format: {value}")]
    InvalidId { value: String },

    #[error("Invalid value for field '{field}': {reason}")]
    InvalidValue { field: String, reason: String },

    // Referential errors
    #[error("Source node not found: {entity_type}/{node_id}")]
    SourceNotFound { entity_type: String, node_id: String },

    #[error("Target node not found: {entity_type}/{node_id}")]
    TargetNotFound { entity_type: String, node_id: String },

    #[error("Node not found: {entity_type}/{node_id}")]
    NodeNotFound { entity_type: String, node_id: String },

    #[error("Edge not found: {source} -[{label}]-> {target}")]
    EdgeNotFound {
        source: String,
        target: String,
        label: String,
    },

    #[error("Target type mismatch for edge '{edge_type}': expected {expected}, got {actual}")]
    TargetTypeMismatch {
        edge_type: String,
        expected: String,
        actual: String,
    },

    #[error("Undeclared edge type '{edge_type}' for entity '{entity_type}'")]
    UndeclaredEdgeType { entity_type: String, edge_type: String },

    // Consistency errors
    #[error("Unique constraint violation: {entity_type}.{field} = {value}")]
    UniqueConstraintViolation {
        entity_type: String,
        field: String,
        value: String,
    },

    #[error("Version conflict for {entity_type}/{node_id}")]
    VersionConflict { entity_type: String, node_id: String },

    #[error("Schema version mismatch: expected {expected}, got {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },

    // Timeout errors
    #[error("Query timeout: {elapsed_ms}ms exceeded {timeout_ms}ms")]
    QueryTimeout { timeout_ms: u64, elapsed_ms: u64 },

    #[error("Traversal timeout at depth {reached_depth}")]
    TraversalTimeout { max_depth: u32, reached_depth: u32 },

    #[error("FDB transaction too old")]
    TransactionTimeout,

    // Resource errors
    #[error("Traversal result limit exceeded: {current_count} >= {max_results}")]
    TraversalLimit { max_results: u32, current_count: u32 },

    // Internal errors
    #[error("FDB unavailable: {0}")]
    FdbUnavailable(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl PelagoError {
    /// Get the error code string
    pub fn code(&self) -> &'static str {
        match self {
            PelagoError::UnregisteredType { .. } => "ERR_VALIDATION_UNREGISTERED_TYPE",
            PelagoError::MissingRequired { .. } => "ERR_VALIDATION_MISSING_REQUIRED",
            PelagoError::TypeMismatch { .. } => "ERR_VALIDATION_TYPE_MISMATCH",
            PelagoError::ExtraProperty { .. } => "ERR_VALIDATION_EXTRA_PROPERTY",
            PelagoError::CelSyntax { .. } => "ERR_VALIDATION_CEL_SYNTAX",
            PelagoError::CelType { .. } => "ERR_VALIDATION_CEL_TYPE",
            PelagoError::InvalidId { .. } => "ERR_VALIDATION_INVALID_ID",
            PelagoError::InvalidValue { .. } => "ERR_VALIDATION_INVALID_VALUE",
            PelagoError::SourceNotFound { .. } => "ERR_REF_SOURCE_NOT_FOUND",
            PelagoError::TargetNotFound { .. } => "ERR_REF_TARGET_NOT_FOUND",
            PelagoError::NodeNotFound { .. } => "ERR_REF_NODE_NOT_FOUND",
            PelagoError::EdgeNotFound { .. } => "ERR_REF_EDGE_NOT_FOUND",
            PelagoError::TargetTypeMismatch { .. } => "ERR_REF_TARGET_TYPE_MISMATCH",
            PelagoError::UndeclaredEdgeType { .. } => "ERR_REF_UNDECLARED_EDGE",
            PelagoError::UniqueConstraintViolation { .. } => "ERR_CONSISTENCY_UNIQUE_VIOLATION",
            PelagoError::VersionConflict { .. } => "ERR_CONSISTENCY_VERSION_CONFLICT",
            PelagoError::SchemaMismatch { .. } => "ERR_CONSISTENCY_SCHEMA_MISMATCH",
            PelagoError::QueryTimeout { .. } => "ERR_TIMEOUT_QUERY",
            PelagoError::TraversalTimeout { .. } => "ERR_TIMEOUT_TRAVERSAL",
            PelagoError::TransactionTimeout => "ERR_TIMEOUT_TRANSACTION",
            PelagoError::TraversalLimit { .. } => "ERR_RESOURCE_TRAVERSAL_LIMIT",
            PelagoError::FdbUnavailable(_) => "ERR_INTERNAL_FDB_UNAVAILABLE",
            PelagoError::Internal(_) => "ERR_INTERNAL_UNEXPECTED",
        }
    }

    /// Get the error category
    pub fn category(&self) -> &'static str {
        match self {
            PelagoError::UnregisteredType { .. }
            | PelagoError::MissingRequired { .. }
            | PelagoError::TypeMismatch { .. }
            | PelagoError::ExtraProperty { .. }
            | PelagoError::CelSyntax { .. }
            | PelagoError::CelType { .. }
            | PelagoError::InvalidId { .. }
            | PelagoError::InvalidValue { .. } => "VALIDATION",

            PelagoError::SourceNotFound { .. }
            | PelagoError::TargetNotFound { .. }
            | PelagoError::NodeNotFound { .. }
            | PelagoError::EdgeNotFound { .. }
            | PelagoError::TargetTypeMismatch { .. }
            | PelagoError::UndeclaredEdgeType { .. } => "REFERENTIAL",

            PelagoError::UniqueConstraintViolation { .. }
            | PelagoError::VersionConflict { .. }
            | PelagoError::SchemaMismatch { .. } => "CONSISTENCY",

            PelagoError::QueryTimeout { .. }
            | PelagoError::TraversalTimeout { .. }
            | PelagoError::TransactionTimeout => "TIMEOUT",

            PelagoError::TraversalLimit { .. } => "RESOURCE",

            PelagoError::FdbUnavailable(_) | PelagoError::Internal(_) => "INTERNAL",
        }
    }

    /// Get metadata for error details
    pub fn metadata(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        match self {
            PelagoError::UnregisteredType { entity_type } => {
                m.insert("entity_type".into(), entity_type.clone());
            }
            PelagoError::MissingRequired { entity_type, field } => {
                m.insert("entity_type".into(), entity_type.clone());
                m.insert("field".into(), field.clone());
            }
            PelagoError::TypeMismatch {
                field,
                expected,
                actual,
            } => {
                m.insert("field".into(), field.clone());
                m.insert("expected".into(), expected.clone());
                m.insert("actual".into(), actual.clone());
            }
            // Add more as needed
            _ => {}
        }
        m
    }
}

// gRPC status code mapping implemented in pelago-api crate
```

**File**: `crates/pelago-core/src/config.rs`
```rust
//! Server configuration

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "pelago", about = "PelagoDB graph database server")]
pub struct ServerConfig {
    /// FDB cluster file path
    #[arg(long, env = "PELAGO_FDB_CLUSTER", default_value = "/etc/foundationdb/fdb.cluster")]
    pub fdb_cluster: String,

    /// Site ID for this server instance (0-255)
    #[arg(long, env = "PELAGO_SITE_ID")]
    pub site_id: u8,

    /// Site name for collision detection
    #[arg(long, env = "PELAGO_SITE_NAME", default_value = "default")]
    pub site_name: String,

    /// gRPC listen address
    #[arg(long, env = "PELAGO_LISTEN_ADDR", default_value = "[::1]:50051")]
    pub listen_addr: String,

    /// Log level filter
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_level: String,

    /// ID allocation batch size
    #[arg(long, env = "PELAGO_ID_BATCH_SIZE", default_value = "100")]
    pub id_batch_size: u64,
}
```

#### 3. Proto Definitions

**File**: `proto/pelago.proto`

The complete proto file as specified in the v1 spec §9.8. This is ~500 lines covering all services and messages. Will be implemented verbatim from the spec.

**File**: `crates/pelago-proto/Cargo.toml`
```toml
[package]
name = "pelago-proto"
version.workspace = true
edition.workspace = true

[dependencies]
prost.workspace = true
prost-types.workspace = true
tonic.workspace = true

[build-dependencies]
tonic-build = "0.12"
```

**File**: `crates/pelago-proto/build.rs`
```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["../../proto/pelago.proto"], &["../../proto"])?;
    Ok(())
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo build` succeeds across all crates
- [ ] `cargo test -p pelago-core` passes all unit tests
- [ ] `NodeId` round-trips through bytes encoding
- [ ] `Value` round-trips through CBOR encoding
- [ ] Sort-order encoding preserves numeric ordering for i64/f64

#### Manual Verification:
- [ ] Docker Compose starts FDB successfully
- [ ] Proto file generates valid Rust code

---

## Phase 1.1: Schema Registry (M1)

### Overview
Implement schema registration, storage, retrieval, and validation. This is the foundation for all data operations.

### Changes Required:

#### 1. Schema Types

**File**: `crates/pelago-core/src/schema.rs`

Define `EntitySchema`, `PropertyDef`, `EdgeDef`, `SchemaMeta`, `IndexType`, `EdgeTarget`, `EdgeDirection`, `OwnershipMode`, `ExtrasPolicy` as specified in §5.1-5.3.

#### 2. Storage Layer Schema Operations

**File**: `crates/pelago-storage/src/schema.rs`

- `register_schema()` - Write schema to FDB, manage version history
- `get_schema()` - Load current schema for entity type
- `get_schema_version()` - Load specific version
- `list_schemas()` - List all schemas in namespace

**FDB Key Layout:**
```
(db, ns, _schema, entity_type, latest) → u32 version
(db, ns, _schema, entity_type, v, version) → CBOR EntitySchema
```

#### 3. Schema Cache

**File**: `crates/pelago-storage/src/cache.rs`

In-memory `HashMap<(String, String, String), Arc<EntitySchema>>` with version checking and invalidation.

### Success Criteria:

#### Automated Verification:
- [ ] Register schema → retrieve → fields match
- [ ] Forward reference edge target → registration succeeds
- [ ] Invalid property type → `SchemaValidation` error
- [ ] Version history is queryable
- [ ] Re-register with incremented version → old version preserved

#### Manual Verification:
- [ ] Schema persists across server restart

---

## Phase 1.2: Node Operations (M2)

### Overview
Implement node CRUD with schema validation, ID allocation, index maintenance, and CDC entry emission.

### Changes Required:

#### 1. ID Allocator

**File**: `crates/pelago-storage/src/ids.rs`

Batch allocation from atomic counter at `(db, ns, _ids, entity_type, site_id)`.

#### 2. Node CRUD

**File**: `crates/pelago-storage/src/node.rs`

- `create_node()` - Validate, allocate ID, write data + indexes + CDC
- `get_node()` - Point lookup by ID
- `update_node()` - Read-modify-write with index diff
- `delete_node()` - Remove data, indexes, cascade edges, emit CDC

#### 3. Index Operations

**File**: `crates/pelago-storage/src/index.rs`

- Unique index: check-then-write pattern
- Equality index: append node_id to value list
- Range index: sort-order encoded values
- Null handling: no index entry for null values

#### 4. CDC Entry Write

**File**: `crates/pelago-storage/src/cdc.rs`

Append `CdcEntry` with versionstamp key in same transaction as mutation.

### Success Criteria:

#### Automated Verification:
- [ ] Create node → ID returned, readable via GetNode
- [ ] Missing required field → rejected
- [ ] Extra field with `extras_policy: reject` → rejected
- [ ] Duplicate unique value → `UniqueConstraintViolation`
- [ ] Update → old index entries removed, new ones added
- [ ] Delete → all data and index entries removed
- [ ] Every mutation has CDC entry in `_cdc` subspace

---

## Phase 1.3: Edge Operations (M3)

### Overview
Implement edge CRUD with bidirectional pairs, referential integrity, and sort key support.

### Changes Required:

**File**: `crates/pelago-storage/src/edge.rs`

- `create_edge()` - Validate nodes exist, write forward/reverse pairs
- `delete_edge()` - Remove all paired entries
- `list_edges()` - Range scan with optional label filter

**FDB Key Layout:**
```
Forward:  (db, ns, edge, f, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id)
Metadata: (db, ns, edge, fm, src_type, src_id, label, [sort_key], tgt_db, tgt_ns, tgt_type, tgt_id)
Reverse:  (db, ns, edge, r, tgt_db, tgt_ns, tgt_type, tgt_id, label, src_type, src_id)
```

### Success Criteria:

#### Automated Verification:
- [ ] Create edge → OUT + IN entries written
- [ ] Edge to non-existent node → rejected
- [ ] Undeclared edge type with `allow_undeclared_edges: false` → rejected
- [ ] Bidirectional edge → 4 entries (2 OUT + 2 IN)
- [ ] Delete bidirectional → all 4 entries removed
- [ ] List edges → sorted by sort key
- [ ] CDC entries for all edge mutations

---

## Phase 1.4: Query System (M4)

### Overview
Implement CEL parsing, type-checking, predicate extraction, index selection, query execution with streaming results, and multi-hop traversal.

**Note:** M4 can start in parallel with M3 since they have independent dependencies.

### Changes Required:

#### 1. CEL Integration

**File**: `crates/pelago-query/src/cel.rs`

- Build type environment from EntitySchema
- Parse CEL → AST
- Type-check against schema
- Null semantics: comparisons return false

#### 2. Query Planning

**File**: `crates/pelago-query/src/planner.rs`

- Extract conjuncts from CEL AST
- Match predicates to available indexes
- Heuristic selectivity: unique=0.01, equality=0.10, range=0.50
- Select most selective index

**File**: `crates/pelago-query/src/plan.rs`

`QueryPlan` enum: `PointLookup`, `RangeScan`, `FullScan`
`ExecutionPlan`: primary plan + residual filter + projection + limit + cursor

#### 3. Query Execution

**File**: `crates/pelago-query/src/executor.rs`

- Execute primary plan against FDB
- Apply residual filter to candidates
- Stream results with backpressure
- Pagination via cursor

#### 4. Snapshot Isolation

Acquire FDB read version at query start, pin all reads to same version.

#### 5. Traversal Engine

**File**: `crates/pelago-query/src/traversal.rs`

- Multi-hop traversal with per-hop edge/node filters
- Depth limit, timeout, result limit enforcement
- Streaming results with path information
- Cycle detection

### Success Criteria:

#### Automated Verification:
- [ ] `FindNodes("Person", "age >= 30")` → range scan, correct results
- [ ] `FindNodes("Person", "email == 'a@b.com'")` → unique index lookup
- [ ] `FindNodes("Person", "bio.contains('X')")` → full scan + residual
- [ ] Multi-predicate → best index + residual
- [ ] Unknown field in CEL → type-check error
- [ ] Null field comparisons return false
- [ ] `age == null` → nodes without age
- [ ] `Explain` returns plan without execution
- [ ] Multi-batch query sees consistent snapshot
- [ ] Traversal depth 4 returns correct paths
- [ ] Traversal respects per-hop filters

---

## Phase 1.5: gRPC API (M5)

### Overview
Wire all storage and query operations into tonic service handlers with proper error mapping.

### Changes Required:

#### 1. Service Handlers

**File**: `crates/pelago-api/src/schema_service.rs`
**File**: `crates/pelago-api/src/node_service.rs`
**File**: `crates/pelago-api/src/edge_service.rs`
**File**: `crates/pelago-api/src/query_service.rs`
**File**: `crates/pelago-api/src/admin_service.rs`
**File**: `crates/pelago-api/src/health_service.rs`

#### 2. Error Mapping

**File**: `crates/pelago-api/src/error.rs`

`PelagoError` → `tonic::Status` with `ErrorDetail` in metadata.

#### 3. Job Storage (No Execution)

**File**: `crates/pelago-storage/src/jobs.rs`

- Job state storage at `(db, ns, _jobs, job_type, job_id)`
- `GetJobStatus` API
- Jobs created in `Pending` state
- **Execution deferred to Phase 2**

#### 4. Server Binary

**File**: `crates/pelago-server/src/main.rs`

- Load config, initialize FDB, validate site ID collision
- Start tonic server with all services
- Graceful shutdown

### Success Criteria:

#### Automated Verification:
- [ ] All RPCs callable via grpcurl
- [ ] `RegisterSchema` → `CreateNode` → `GetNode` → `FindNodes` works
- [ ] Streaming endpoints deliver results correctly
- [ ] Error responses include `ErrorDetail` in metadata
- [ ] HealthService.Check returns SERVING

---

## Phase 1.6: Integration Testing (M6)

### Overview
Comprehensive test suite at both Rust integration and Python gRPC levels.

### Changes Required:

**Directory**: `tests/integration/`

Rust tests against FDB internals: schema_tests.rs, node_tests.rs, edge_tests.rs, query_tests.rs, cdc_tests.rs, traversal_tests.rs

**Directory**: `tests/python/`

Python gRPC contract tests: conftest.py, test_schema.py, test_nodes.py, test_edges.py, test_queries.py, test_admin.py, test_lifecycle.py

### Success Criteria:

#### Automated Verification:
- [ ] `cargo test --test integration` passes
- [ ] `pytest tests/python` passes against running server
- [ ] Point lookup < 1ms p99
- [ ] Index query < 10ms p99
- [ ] Traversal (depth 4, 100 nodes) < 100ms p99

#### Manual Verification:
- [ ] Full workflow tested: schema → nodes → edges → query → cleanup
- [ ] No data corruption under concurrent operations

---

## Testing Strategy

### Unit Tests
- Core types: NodeId, EdgeId, Value encoding roundtrips
- Encoding: sort-order preservation for numerics
- Schema validation logic

### Integration Tests (Rust)
- FDB key layout verification
- Transaction atomicity
- Index correctness
- CDC entry completeness

### API Tests (Python)
- Full gRPC contract validation
- Streaming endpoint behavior
- Error code verification
- Pagination and cursors

### Performance Tests
- Point lookup latency
- Index query throughput
- Traversal performance

---

## Performance Considerations

1. **ID Allocation Batching**: Grab blocks of 100 IDs to reduce FDB transaction overhead
2. **Schema Caching**: In-memory cache with version check to avoid FDB reads
3. **Index Selection Heuristics**: Use selectivity estimates to choose best index
4. **Snapshot Reads**: Pin read version at query start for consistency
5. **Streaming Results**: Backpressure-aware streaming for large result sets
6. **5-Second Transaction Window**: Paginate long queries to stay within FDB limits

---

## Migration Notes

N/A - This is a greenfield project.

---

## References

- **v1 Spec**: `.llm/context/pelagodb-spec-v1.md` (sections 1-9, 13-16)
- **Implementation Phases**: `.llm/shared/research/2026-02-14-pelagodb-implementation-phases.md`
- **Archived Phase 1 Plan**: `.llm/context/archive/phase-1-plan.md`
- **Critique Document**: `.llm/shared/research/2026-02-14-implementation-phases-critique.md`

---

## File Summary

### New Files to Create

```
pelago/
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml
├── proto/
│   └── pelago.proto
├── crates/
│   ├── pelago-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs
│   │       ├── encoding.rs
│   │       ├── schema.rs
│   │       ├── errors.rs
│   │       └── config.rs
│   ├── pelago-proto/
│   │   ├── Cargo.toml
│   │   ├── build.rs
│   │   └── src/lib.rs
│   ├── pelago-storage/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── subspace.rs
│   │       ├── schema.rs
│   │       ├── node.rs
│   │       ├── edge.rs
│   │       ├── index.rs
│   │       ├── cdc.rs
│   │       ├── ids.rs
│   │       ├── jobs.rs
│   │       └── cache.rs
│   ├── pelago-query/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── cel.rs
│   │       ├── planner.rs
│   │       ├── executor.rs
│   │       ├── plan.rs
│   │       └── traversal.rs
│   ├── pelago-api/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── schema_service.rs
│   │       ├── node_service.rs
│   │       ├── edge_service.rs
│   │       ├── query_service.rs
│   │       ├── admin_service.rs
│   │       ├── health_service.rs
│   │       └── error.rs
│   └── pelago-server/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
└── tests/
    ├── integration/
    │   ├── schema_tests.rs
    │   ├── node_tests.rs
    │   ├── edge_tests.rs
    │   ├── query_tests.rs
    │   ├── cdc_tests.rs
    │   └── traversal_tests.rs
    └── python/
        ├── requirements.txt
        ├── generate_stubs.sh
        ├── conftest.py
        ├── test_schema.py
        ├── test_nodes.py
        ├── test_edges.py
        ├── test_queries.py
        ├── test_admin.py
        └── test_lifecycle.py
```

**Total: ~40 files, 7 crates**

---

## Changelog

- **2026-02-14 (v1)**: Initial implementation plan created from v1 spec and existing research documents
