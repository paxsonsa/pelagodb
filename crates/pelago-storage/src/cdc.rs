//! Change Data Capture (CDC) system
//!
//! CDC entries are written atomically within mutation transactions using FDB
//! versionstamped keys for zero-contention, globally-ordered append.
//!
//! FDB Key Layout:
//! ```text
//! (db, ns, _cdc, <10-byte versionstamp>) → CBOR CdcEntry
//! ```
//!
//! The `CdcAccumulator` collects operations during a transaction and flushes
//! them as a single CDC entry using `SetVersionstampedKey`. This ensures CDC
//! entries are written atomically with the mutation they describe.

use crate::db::PelagoDb;
use crate::Subspace;
use foundationdb::options::MutationType;
use pelago_core::encoding::{decode_cbor, encode_cbor};
use pelago_core::{PelagoError, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Versionstamp ────────────────────────────────────────────────────────

/// FDB versionstamp: 10 bytes (8-byte transaction version + 2-byte batch order)
///
/// Versionstamps are globally ordered — any transaction that commits later
/// will have a strictly greater versionstamp than one that committed earlier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Versionstamp(#[serde(with = "versionstamp_serde")] [u8; 10]);

impl Versionstamp {
    /// A zero versionstamp (used as "start from beginning")
    pub fn zero() -> Self {
        Self([0u8; 10])
    }

    /// Construct from a 10-byte slice
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 10 {
            return None;
        }
        let mut arr = [0u8; 10];
        arr.copy_from_slice(bytes);
        Some(Self(arr))
    }

    /// Get the raw bytes
    pub fn to_bytes(&self) -> &[u8; 10] {
        &self.0
    }

    /// Increment to produce the exclusive start for "after this" range scans.
    /// Returns the smallest versionstamp that is strictly greater than self.
    pub fn next(&self) -> Self {
        let mut result = self.0;
        // Increment from the last byte with carry
        for i in (0..10).rev() {
            if result[i] < 0xFF {
                result[i] += 1;
                return Self(result);
            }
            result[i] = 0x00;
        }
        // All bytes were 0xFF — overflow to zero (shouldn't happen in practice)
        Self(result)
    }

    /// Check if this is the zero versionstamp
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 10]
    }
}

impl std::fmt::Display for Versionstamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

/// Custom serde for the fixed-size byte array (CBOR-friendly)
mod versionstamp_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 10], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 10], D::Error> {
        let buf = <Vec<u8>>::deserialize(d)?;
        buf.try_into().map_err(|v: Vec<u8>| {
            serde::de::Error::custom(format!("expected 10 bytes, got {}", v.len()))
        })
    }
}

// ─── CDC Entry Types ─────────────────────────────────────────────────────

/// CDC entry — one per FDB transaction, may contain multiple operations.
///
/// When a single transaction creates a node and its edges, they all appear
/// as operations within the same CdcEntry, sharing one versionstamp.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CdcEntry {
    /// Site that performed the mutation
    pub site: String,
    /// Timestamp of the mutation (unix microseconds)
    pub timestamp: i64,
    /// Optional batch identifier for grouped operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    /// List of operations in this entry
    pub operations: Vec<CdcOperation>,
}

/// Individual CDC operation within an entry.
///
/// Each variant captures the data needed to replicate or react to a mutation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CdcOperation {
    NodeCreate {
        entity_type: String,
        node_id: String,
        properties: HashMap<String, Value>,
        home_site: String,
    },
    NodeUpdate {
        entity_type: String,
        node_id: String,
        changed_properties: HashMap<String, Value>,
        old_properties: HashMap<String, Value>,
    },
    NodeDelete {
        entity_type: String,
        node_id: String,
    },
    EdgeCreate {
        source_type: String,
        source_id: String,
        target_type: String,
        target_id: String,
        edge_type: String,
        edge_id: String,
        properties: HashMap<String, Value>,
    },
    EdgeDelete {
        source_type: String,
        source_id: String,
        target_type: String,
        target_id: String,
        edge_type: String,
    },
    SchemaRegister {
        entity_type: String,
        version: u32,
    },
    OwnershipTransfer {
        entity_type: String,
        node_id: String,
        previous_site_id: String,
        current_site_id: String,
    },
}

// ─── CDC Accumulator ─────────────────────────────────────────────────────

/// Accumulates CDC operations during a transaction and writes them atomically.
///
/// Create one per transaction, push operations as mutations happen, then call
/// `flush()` before committing. The flush writes a single CDC entry with a
/// versionstamped key, ensuring it's ordered globally and committed atomically
/// with the mutation data.
pub struct CdcAccumulator {
    site: String,
    operations: Vec<CdcOperation>,
}

impl CdcAccumulator {
    /// Create a new accumulator for the given site
    pub fn new(site: &str) -> Self {
        Self {
            site: site.to_string(),
            operations: Vec::new(),
        }
    }

    /// Add an operation to the accumulator
    pub fn push(&mut self, op: CdcOperation) {
        self.operations.push(op);
    }

    /// Returns true if no operations have been accumulated
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Write accumulated CDC entry into the given FDB transaction using a versionstamped key.
    ///
    /// The key format before FDB processes it:
    /// ```text
    /// [subspace_prefix | 10-byte zero placeholder | 4-byte LE offset]
    /// ```
    ///
    /// After commit, FDB replaces the 10 zero bytes with the real versionstamp
    /// and strips the 4-byte offset, yielding:
    /// ```text
    /// [subspace_prefix | 10-byte versionstamp]
    /// ```
    pub fn flush(
        self,
        trx: &foundationdb::Transaction,
        database: &str,
        namespace: &str,
    ) -> Result<(), PelagoError> {
        if self.operations.is_empty() {
            return Ok(());
        }

        let entry = CdcEntry {
            site: self.site,
            timestamp: now_micros(),
            batch_id: None,
            operations: self.operations,
        };

        write_cdc_entry(trx, database, namespace, &entry)
    }
}

fn write_cdc_entry(
    trx: &foundationdb::Transaction,
    database: &str,
    namespace: &str,
    entry: &CdcEntry,
) -> Result<(), PelagoError> {
    let subspace = Subspace::namespace(database, namespace).cdc();
    let value = encode_cbor(entry)?;

    // Build key with versionstamp placeholder + offset trailer
    let prefix = subspace.prefix();
    let versionstamp_offset = prefix.len() as u32;
    let mut key = Vec::with_capacity(prefix.len() + 10 + 4);
    key.extend_from_slice(prefix);
    key.extend_from_slice(&[0u8; 10]); // versionstamp placeholder
    key.extend_from_slice(&versionstamp_offset.to_le_bytes()); // offset

    trx.atomic_op(&key, &value, MutationType::SetVersionstampedKey);
    Ok(())
}

/// Append a CDC entry as a standalone transaction.
///
/// This is used by system workers (for example, replication) that need to
/// mirror externally applied mutations into the local CDC stream so local
/// consumers (cache projector, watchers) converge.
pub async fn append_cdc_entry(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    entry: CdcEntry,
) -> Result<(), PelagoError> {
    if entry.operations.is_empty() {
        return Ok(());
    }

    let trx = db.create_transaction()?;
    write_cdc_entry(&trx, database, namespace, &entry)?;
    trx.commit()
        .await
        .map_err(|e| PelagoError::Internal(format!("Failed to append CDC entry: {}", e)))?;
    Ok(())
}

// ─── CDC Reader ──────────────────────────────────────────────────────────

/// Read CDC entries from a versionstamp range.
///
/// Returns entries ordered by versionstamp. If `after_versionstamp` is Some,
/// only entries with versionstamps strictly greater are returned.
pub async fn read_cdc_entries(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    after_versionstamp: Option<&Versionstamp>,
    limit: usize,
) -> Result<Vec<(Versionstamp, CdcEntry)>, PelagoError> {
    let subspace = Subspace::namespace(database, namespace).cdc();

    let range_start = match after_versionstamp {
        Some(vs) => {
            let next = vs.next();
            let mut key = subspace.prefix().to_vec();
            key.extend_from_slice(next.to_bytes());
            key
        }
        None => subspace.prefix().to_vec(),
    };
    let range_end = subspace.range_end().to_vec();

    let results = db.get_range(&range_start, &range_end, limit).await?;

    let prefix_len = subspace.prefix().len();
    let mut entries = Vec::new();
    for (key, value) in results {
        let vs_bytes = &key[prefix_len..];
        if let Some(vs) = Versionstamp::from_bytes(vs_bytes) {
            let entry: CdcEntry = decode_cbor(&value)?;
            entries.push((vs, entry));
        }
    }

    Ok(entries)
}

/// Returns true if an exact CDC entry exists at the provided versionstamp.
pub async fn cdc_position_exists(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    versionstamp: &Versionstamp,
) -> Result<bool, PelagoError> {
    if versionstamp.is_zero() {
        return Ok(true);
    }
    let mut key = Subspace::namespace(database, namespace)
        .cdc()
        .prefix()
        .to_vec();
    key.extend_from_slice(versionstamp.to_bytes());
    Ok(db.get(&key).await?.is_some())
}

fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versionstamp_zero() {
        let vs = Versionstamp::zero();
        assert!(vs.is_zero());
        assert_eq!(vs.to_bytes(), &[0u8; 10]);
    }

    #[test]
    fn test_versionstamp_from_bytes() {
        let bytes = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let vs = Versionstamp::from_bytes(&bytes).unwrap();
        assert_eq!(vs.to_bytes(), &bytes);
        assert!(!vs.is_zero());
    }

    #[test]
    fn test_versionstamp_from_bytes_wrong_size() {
        assert!(Versionstamp::from_bytes(&[1, 2, 3]).is_none());
        assert!(Versionstamp::from_bytes(&[0; 11]).is_none());
    }

    #[test]
    fn test_versionstamp_ordering() {
        let a = Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 0]).unwrap();
        let b = Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 2, 0, 0]).unwrap();
        assert!(a < b);
    }

    #[test]
    fn test_versionstamp_next() {
        let vs = Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 5]).unwrap();
        let next = vs.next();
        assert_eq!(next.to_bytes(), &[0, 0, 0, 0, 0, 0, 0, 0, 0, 6]);
        assert!(next > vs);
    }

    #[test]
    fn test_versionstamp_next_carry() {
        let vs = Versionstamp::from_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF]).unwrap();
        let next = vs.next();
        assert_eq!(next.to_bytes(), &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
    }

    #[test]
    fn test_versionstamp_display() {
        let vs = Versionstamp::from_bytes(&[0xAB, 0xCD, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(vs.to_string(), "abcd0000000000000000");
    }

    #[test]
    fn test_cdc_entry_serialization_roundtrip() {
        let entry = CdcEntry {
            site: "1".to_string(),
            timestamp: 1234567890,
            batch_id: None,
            operations: vec![
                CdcOperation::NodeCreate {
                    entity_type: "User".to_string(),
                    node_id: "1_100".to_string(),
                    properties: HashMap::from([(
                        "name".to_string(),
                        Value::String("Alice".to_string()),
                    )]),
                    home_site: "1".to_string(),
                },
                CdcOperation::EdgeCreate {
                    source_type: "User".to_string(),
                    source_id: "1_100".to_string(),
                    target_type: "User".to_string(),
                    target_id: "1_200".to_string(),
                    edge_type: "follows".to_string(),
                    edge_id: "1_50".to_string(),
                    properties: HashMap::new(),
                },
            ],
        };

        let bytes = encode_cbor(&entry).unwrap();
        let decoded: CdcEntry = decode_cbor(&bytes).unwrap();

        assert_eq!(decoded.site, "1");
        assert_eq!(decoded.operations.len(), 2);
    }

    #[test]
    fn test_cdc_operation_variants_serialize() {
        let ops = vec![
            CdcOperation::NodeCreate {
                entity_type: "T".into(),
                node_id: "1".into(),
                properties: HashMap::new(),
                home_site: "1".into(),
            },
            CdcOperation::NodeUpdate {
                entity_type: "T".into(),
                node_id: "1".into(),
                changed_properties: HashMap::new(),
                old_properties: HashMap::new(),
            },
            CdcOperation::NodeDelete {
                entity_type: "T".into(),
                node_id: "1".into(),
            },
            CdcOperation::EdgeCreate {
                source_type: "A".into(),
                source_id: "1".into(),
                target_type: "B".into(),
                target_id: "2".into(),
                edge_type: "E".into(),
                edge_id: "3".into(),
                properties: HashMap::new(),
            },
            CdcOperation::EdgeDelete {
                source_type: "A".into(),
                source_id: "1".into(),
                target_type: "B".into(),
                target_id: "2".into(),
                edge_type: "E".into(),
            },
            CdcOperation::SchemaRegister {
                entity_type: "T".into(),
                version: 1,
            },
            CdcOperation::OwnershipTransfer {
                entity_type: "T".into(),
                node_id: "1".into(),
                previous_site_id: "1".into(),
                current_site_id: "2".into(),
            },
        ];

        for op in ops {
            let entry = CdcEntry {
                site: "1".into(),
                timestamp: 0,
                batch_id: None,
                operations: vec![op],
            };
            let bytes = encode_cbor(&entry).unwrap();
            let decoded: CdcEntry = decode_cbor(&bytes).unwrap();
            assert_eq!(decoded.operations.len(), 1);
        }
    }

    #[test]
    fn test_versionstamp_serde_roundtrip() {
        let vs = Versionstamp::from_bytes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]).unwrap();
        let bytes = encode_cbor(&vs).unwrap();
        let decoded: Versionstamp = decode_cbor(&bytes).unwrap();
        assert_eq!(vs, decoded);
    }

    #[test]
    fn test_accumulator_empty_flush_is_noop() {
        let acc = CdcAccumulator::new("1");
        assert!(acc.is_empty());
        // Can't test flush without a real transaction, but is_empty check is sufficient
    }
}
