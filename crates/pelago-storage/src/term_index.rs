//! Generic term posting index for boolean predicate execution.
//!
//! Key layout in the `idx` subspace:
//! - Posting: (db, ns, idx, entity_type, 't', field, encoded_value, node_id) -> empty
//! - DF stat: (db, ns, idx, entity_type, 'd', field, encoded_value) -> i64 be
//! - NDocs:   (db, ns, idx, entity_type, 'n') -> i64 be
//!
//! This index is internal and can be used by any entity type.

use crate::{PelagoDb, Subspace};
use bytes::Bytes;
use foundationdb::Transaction;
use pelago_core::encoding::encode_value_for_index;
use pelago_core::{PelagoError, Value};
use std::collections::{HashMap, HashSet};

pub mod markers {
    pub const TERM_POSTING: u8 = b't';
    pub const DF_STAT: u8 = b'd';
    pub const NDOCS_STAT: u8 = b'n';
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TermKey {
    pub field: String,
    pub encoded_value: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TermPosting {
    pub key: Bytes,
    pub term: TermKey,
}

#[derive(Debug, Clone, Default)]
pub struct TermPostingChanges {
    pub additions: Vec<TermPosting>,
    pub removals: Vec<TermPosting>,
}

pub fn compute_term_postings(
    subspace: &Subspace,
    entity_type: &str,
    node_id: &[u8],
    properties: &HashMap<String, Value>,
) -> Result<Vec<TermPosting>, PelagoError> {
    let idx_subspace = subspace.index();
    let mut postings = Vec::new();

    for (field, value) in properties {
        let Some(encoded) = encode_term_value(value)? else {
            continue;
        };

        let key = idx_subspace
            .pack()
            .add_string(entity_type)
            .add_marker(markers::TERM_POSTING)
            .add_string(field)
            .add_bytes(&encoded)
            .add_raw_bytes(node_id)
            .build();

        postings.push(TermPosting {
            key,
            term: TermKey {
                field: field.clone(),
                encoded_value: encoded,
            },
        });
    }

    Ok(postings)
}

pub fn compute_term_posting_changes(
    subspace: &Subspace,
    entity_type: &str,
    node_id: &[u8],
    old_properties: &HashMap<String, Value>,
    new_properties: &HashMap<String, Value>,
) -> Result<TermPostingChanges, PelagoError> {
    let mut additions = Vec::new();
    let mut removals = Vec::new();
    let idx_subspace = subspace.index();

    let fields: HashSet<&str> = old_properties
        .keys()
        .map(String::as_str)
        .chain(new_properties.keys().map(String::as_str))
        .collect();

    for field in fields {
        let old_value = old_properties.get(field);
        let new_value = new_properties.get(field);

        let old_encoded = old_value.map(encode_term_value).transpose()?.flatten();
        let new_encoded = new_value.map(encode_term_value).transpose()?.flatten();

        match (old_encoded, new_encoded) {
            (Some(old), Some(new)) if old != new => {
                removals.push(build_posting(
                    &idx_subspace,
                    entity_type,
                    field,
                    &old,
                    node_id,
                ));
                additions.push(build_posting(
                    &idx_subspace,
                    entity_type,
                    field,
                    &new,
                    node_id,
                ));
            }
            (Some(old), None) => {
                removals.push(build_posting(
                    &idx_subspace,
                    entity_type,
                    field,
                    &old,
                    node_id,
                ));
            }
            (None, Some(new)) => {
                additions.push(build_posting(
                    &idx_subspace,
                    entity_type,
                    field,
                    &new,
                    node_id,
                ));
            }
            _ => {}
        }
    }

    Ok(TermPostingChanges {
        additions,
        removals,
    })
}

pub async fn apply_term_posting_changes(
    trx: &Transaction,
    subspace: &Subspace,
    entity_type: &str,
    changes: &TermPostingChanges,
    n_docs_delta: i64,
) -> Result<(), PelagoError> {
    for posting in &changes.removals {
        trx.clear(posting.key.as_ref());
    }
    for posting in &changes.additions {
        trx.set(posting.key.as_ref(), &[]);
    }

    let mut term_deltas: HashMap<TermKey, i64> = HashMap::new();
    for posting in &changes.additions {
        *term_deltas.entry(posting.term.clone()).or_insert(0) += 1;
    }
    for posting in &changes.removals {
        *term_deltas.entry(posting.term.clone()).or_insert(0) -= 1;
    }

    let idx_subspace = subspace.index();
    for (term, delta) in term_deltas {
        if delta == 0 {
            continue;
        }
        let key = df_key(&idx_subspace, entity_type, &term.field, &term.encoded_value);
        let current = read_i64_counter(trx, key.as_ref()).await?;
        let next = (current + delta).max(0);
        if next == 0 {
            trx.clear(key.as_ref());
        } else {
            trx.set(key.as_ref(), &next.to_be_bytes());
        }
    }

    if n_docs_delta != 0 {
        let key = n_docs_key(&idx_subspace, entity_type);
        let current = read_i64_counter(trx, key.as_ref()).await?;
        let next = (current + n_docs_delta).max(0);
        if next == 0 {
            trx.clear(key.as_ref());
        } else {
            trx.set(key.as_ref(), &next.to_be_bytes());
        }
    }

    Ok(())
}

pub async fn list_posting_node_ids(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    entity_type: &str,
    field: &str,
    value: &Value,
    limit: usize,
) -> Result<Vec<Vec<u8>>, PelagoError> {
    list_posting_node_ids_after(
        db,
        database,
        namespace,
        entity_type,
        field,
        value,
        None,
        limit,
    )
    .await
}

pub async fn list_posting_node_ids_after(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    entity_type: &str,
    field: &str,
    value: &Value,
    after_node_id: Option<&[u8]>,
    limit: usize,
) -> Result<Vec<Vec<u8>>, PelagoError> {
    let Some(encoded) = encode_term_value(value)? else {
        return Ok(Vec::new());
    };
    let idx_subspace = Subspace::namespace(database, namespace).index();
    let prefix = posting_prefix(&idx_subspace, entity_type, field, &encoded);
    let range_start = if let Some(node_id) = after_node_id {
        make_posting_start_after(&prefix, node_id)
    } else {
        prefix.to_vec()
    };
    let mut end = prefix.to_vec();
    end.push(0xFF);

    let rows = db.get_range(&range_start, &end, limit.max(1)).await?;
    let mut out = Vec::with_capacity(rows.len());
    for (key, _) in rows {
        out.push(extract_node_id_from_posting_key(&key)?);
    }
    Ok(out)
}

pub async fn get_document_frequency(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    entity_type: &str,
    field: &str,
    value: &Value,
) -> Result<Option<u64>, PelagoError> {
    let Some(encoded) = encode_term_value(value)? else {
        return Ok(Some(0));
    };
    let idx_subspace = Subspace::namespace(database, namespace).index();
    let key = df_key(&idx_subspace, entity_type, field, &encoded);
    match db.get(key.as_ref()).await? {
        Some(bytes) => {
            if bytes.len() != 8 {
                return Err(PelagoError::Internal(format!(
                    "Invalid term DF counter length: {}",
                    bytes.len()
                )));
            }
            let arr: [u8; 8] = bytes.try_into().unwrap();
            let n = i64::from_be_bytes(arr).max(0) as u64;
            Ok(Some(n))
        }
        None => Ok(None),
    }
}

pub async fn get_document_count(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    entity_type: &str,
) -> Result<Option<u64>, PelagoError> {
    let idx_subspace = Subspace::namespace(database, namespace).index();
    let key = n_docs_key(&idx_subspace, entity_type);
    match db.get(key.as_ref()).await? {
        Some(bytes) => {
            if bytes.len() != 8 {
                return Err(PelagoError::Internal(format!(
                    "Invalid n_docs counter length: {}",
                    bytes.len()
                )));
            }
            let arr: [u8; 8] = bytes.try_into().unwrap();
            let n = i64::from_be_bytes(arr).max(0) as u64;
            Ok(Some(n))
        }
        None => Ok(None),
    }
}

fn posting_prefix(
    idx_subspace: &Subspace,
    entity_type: &str,
    field: &str,
    encoded_value: &[u8],
) -> Bytes {
    idx_subspace
        .pack()
        .add_string(entity_type)
        .add_marker(markers::TERM_POSTING)
        .add_string(field)
        .add_bytes(encoded_value)
        .build()
}

fn df_key(idx_subspace: &Subspace, entity_type: &str, field: &str, encoded_value: &[u8]) -> Bytes {
    idx_subspace
        .pack()
        .add_string(entity_type)
        .add_marker(markers::DF_STAT)
        .add_string(field)
        .add_bytes(encoded_value)
        .build()
}

fn n_docs_key(idx_subspace: &Subspace, entity_type: &str) -> Bytes {
    idx_subspace
        .pack()
        .add_string(entity_type)
        .add_marker(markers::NDOCS_STAT)
        .build()
}

fn build_posting(
    idx_subspace: &Subspace,
    entity_type: &str,
    field: &str,
    encoded_value: &[u8],
    node_id: &[u8],
) -> TermPosting {
    let key = idx_subspace
        .pack()
        .add_string(entity_type)
        .add_marker(markers::TERM_POSTING)
        .add_string(field)
        .add_bytes(encoded_value)
        .add_raw_bytes(node_id)
        .build();

    TermPosting {
        key,
        term: TermKey {
            field: field.to_string(),
            encoded_value: encoded_value.to_vec(),
        },
    }
}

fn encode_term_value(value: &Value) -> Result<Option<Vec<u8>>, PelagoError> {
    match value {
        Value::Null => Ok(None),
        Value::Bytes(_) => Ok(None),
        _ => Ok(Some(encode_value_for_index(value)?)),
    }
}

async fn read_i64_counter(trx: &Transaction, key: &[u8]) -> Result<i64, PelagoError> {
    match trx
        .get(key, false)
        .await
        .map_err(|e| PelagoError::Internal(format!("Failed reading term counter: {}", e)))?
    {
        Some(bytes) => {
            if bytes.len() != 8 {
                return Err(PelagoError::Internal(format!(
                    "Invalid term counter length: {}",
                    bytes.len()
                )));
            }
            let arr: [u8; 8] = bytes.as_ref().try_into().unwrap();
            Ok(i64::from_be_bytes(arr))
        }
        None => Ok(0),
    }
}

fn extract_node_id_from_posting_key(key: &[u8]) -> Result<Vec<u8>, PelagoError> {
    if key.len() < 9 {
        return Err(PelagoError::Internal("Term posting key too short".into()));
    }
    Ok(key[key.len() - 9..].to_vec())
}

fn make_posting_start_after(prefix: &[u8], node_id: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + node_id.len() + 1);
    out.extend_from_slice(prefix);
    out.extend_from_slice(node_id);
    out.push(0x00);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelago_core::NodeId;

    #[test]
    fn test_compute_term_postings_skips_null_and_bytes() {
        let subspace = Subspace::namespace("db", "ns");
        let node_id = NodeId::new(1, 42).to_bytes();
        let mut props = HashMap::new();
        props.insert("name".into(), Value::String("alice".into()));
        props.insert("age".into(), Value::Int(30));
        props.insert("blob".into(), Value::Bytes(vec![1, 2, 3]));
        props.insert("maybe".into(), Value::Null);

        let postings = compute_term_postings(&subspace, "Person", &node_id, &props).unwrap();
        assert_eq!(postings.len(), 2);
    }

    #[test]
    fn test_compute_term_posting_changes() {
        let subspace = Subspace::namespace("db", "ns");
        let node_id = NodeId::new(1, 7).to_bytes();
        let mut old_props = HashMap::new();
        old_props.insert("task".into(), Value::String("fx".into()));
        old_props.insert("label".into(), Value::String("default".into()));
        let mut new_props = old_props.clone();
        new_props.insert("task".into(), Value::String("anim".into()));
        new_props.remove("label");
        new_props.insert("shot".into(), Value::String("shot001".into()));

        let changes =
            compute_term_posting_changes(&subspace, "Context", &node_id, &old_props, &new_props)
                .unwrap();
        assert_eq!(changes.additions.len(), 2);
        assert_eq!(changes.removals.len(), 2);
    }
}
