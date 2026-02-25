use super::filter::query_matches_node;
use super::state::node_key;
use super::{WatchContext, WatchFilter};
use crate::schema_service::core_to_proto_properties;
use pelago_core::encoding::decode_cbor;
use pelago_core::{NodeId, Value};
use pelago_proto::{WatchEvent, WatchEventType};
use pelago_storage::{PelagoDb, SchemaRegistry, Subspace};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::Status;

#[derive(Debug, Clone, Deserialize)]
struct SnapshotNodeData {
    properties: HashMap<String, Value>,
    locality: u8,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct SnapshotNode {
    entity_type: String,
    node_id: String,
    data: SnapshotNodeData,
}

pub(super) async fn emit_initial_snapshot(
    db: &PelagoDb,
    schema_registry: &Arc<SchemaRegistry>,
    ctx: &WatchContext,
    filter: &WatchFilter,
    tx: &mpsc::Sender<Result<WatchEvent, Status>>,
    matched_nodes: &mut HashSet<String>,
) -> Result<(), Status> {
    match filter {
        WatchFilter::Point { nodes, .. } => {
            let mut emitted = 0usize;
            for node_target in nodes {
                if emitted >= ctx.initial_snapshot_limit.max(1) {
                    break;
                }
                if let Some(snapshot) = fetch_node_snapshot(
                    db,
                    &ctx.database,
                    &ctx.namespace,
                    &node_target.entity_type,
                    &node_target.node_id,
                )
                .await?
                {
                    let mut properties = snapshot.data.properties.clone();
                    if !node_target.properties.is_empty() {
                        properties.retain(|k, _| node_target.properties.contains(k));
                    }
                    tx.send(Ok(WatchEvent {
                        subscription_id: ctx.subscription_id.clone(),
                        r#type: WatchEventType::Enter as i32,
                        versionstamp: ctx.after.to_bytes().to_vec(),
                        node: Some(pelago_proto::Node {
                            id: snapshot.node_id.clone(),
                            entity_type: snapshot.entity_type.clone(),
                            properties: core_to_proto_properties(&properties),
                            locality: snapshot.data.locality.to_string(),
                            created_at: snapshot.data.created_at,
                            updated_at: snapshot.data.updated_at,
                        }),
                        edge: None,
                        reason: "initial_snapshot".to_string(),
                    }))
                    .await
                    .map_err(|_| Status::cancelled("watch stream closed"))?;
                    emitted += 1;
                }
            }
        }
        WatchFilter::Query(query_filter) => {
            let mut remaining = ctx.initial_snapshot_limit.max(1);
            let entity_types: Vec<String> = if !query_filter.entity_type.is_empty() {
                vec![query_filter.entity_type.clone()]
            } else if let Some(root) = query_filter
                .pql_predicate
                .as_ref()
                .and_then(|p| p.root_entity_type.clone())
            {
                vec![root]
            } else {
                schema_registry
                    .list_schemas(&ctx.database, &ctx.namespace)
                    .await
                    .map_err(|e| Status::internal(format!("snapshot list schemas failed: {}", e)))?
            };

            for entity_type in entity_types {
                if remaining == 0 {
                    break;
                }
                let snapshots = scan_entity_snapshots(
                    db,
                    &ctx.database,
                    &ctx.namespace,
                    &entity_type,
                    remaining,
                )
                .await?;
                for snapshot in snapshots {
                    let matched = query_matches_node(
                        query_filter,
                        &ctx.database,
                        &ctx.namespace,
                        &snapshot.entity_type,
                        &snapshot.data.properties,
                        schema_registry,
                    )
                    .await;
                    if !matched {
                        continue;
                    }
                    matched_nodes.insert(node_key(&snapshot.entity_type, &snapshot.node_id));
                    tx.send(Ok(WatchEvent {
                        subscription_id: ctx.subscription_id.clone(),
                        r#type: WatchEventType::Enter as i32,
                        versionstamp: ctx.after.to_bytes().to_vec(),
                        node: Some(pelago_proto::Node {
                            id: snapshot.node_id.clone(),
                            entity_type: snapshot.entity_type.clone(),
                            properties: core_to_proto_properties(&snapshot.data.properties),
                            locality: snapshot.data.locality.to_string(),
                            created_at: snapshot.data.created_at,
                            updated_at: snapshot.data.updated_at,
                        }),
                        edge: None,
                        reason: "initial_snapshot".to_string(),
                    }))
                    .await
                    .map_err(|_| Status::cancelled("watch stream closed"))?;
                    remaining = remaining.saturating_sub(1);
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
        WatchFilter::Namespace => {
            let mut remaining = ctx.initial_snapshot_limit.max(1);
            let entity_types = schema_registry
                .list_schemas(&ctx.database, &ctx.namespace)
                .await
                .map_err(|e| Status::internal(format!("snapshot list schemas failed: {}", e)))?;
            for entity_type in entity_types {
                if remaining == 0 {
                    break;
                }
                let snapshots = scan_entity_snapshots(
                    db,
                    &ctx.database,
                    &ctx.namespace,
                    &entity_type,
                    remaining,
                )
                .await?;
                for snapshot in snapshots {
                    tx.send(Ok(WatchEvent {
                        subscription_id: ctx.subscription_id.clone(),
                        r#type: WatchEventType::Enter as i32,
                        versionstamp: ctx.after.to_bytes().to_vec(),
                        node: Some(pelago_proto::Node {
                            id: snapshot.node_id,
                            entity_type: snapshot.entity_type,
                            properties: core_to_proto_properties(&snapshot.data.properties),
                            locality: snapshot.data.locality.to_string(),
                            created_at: snapshot.data.created_at,
                            updated_at: snapshot.data.updated_at,
                        }),
                        edge: None,
                        reason: "initial_snapshot".to_string(),
                    }))
                    .await
                    .map_err(|_| Status::cancelled("watch stream closed"))?;
                    remaining = remaining.saturating_sub(1);
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn fetch_node_snapshot(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    entity_type: &str,
    node_id: &str,
) -> Result<Option<SnapshotNode>, Status> {
    let parsed: NodeId = node_id
        .parse()
        .map_err(|_| Status::invalid_argument("invalid node_id in watch target"))?;
    let node_bytes = parsed.to_bytes();
    let key = Subspace::namespace(database, namespace)
        .data()
        .pack()
        .add_string(entity_type)
        .add_raw_bytes(&node_bytes)
        .build();
    let Some(value) = db
        .get(key.as_ref())
        .await
        .map_err(|e| Status::internal(format!("snapshot get failed: {}", e)))?
    else {
        return Ok(None);
    };

    let data: SnapshotNodeData = decode_cbor(&value)
        .map_err(|e| Status::internal(format!("snapshot decode failed: {}", e)))?;
    Ok(Some(SnapshotNode {
        entity_type: entity_type.to_string(),
        node_id: node_id.to_string(),
        data,
    }))
}

async fn scan_entity_snapshots(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    entity_type: &str,
    limit: usize,
) -> Result<Vec<SnapshotNode>, Status> {
    let prefix = Subspace::namespace(database, namespace)
        .data()
        .pack()
        .add_string(entity_type)
        .build();
    let mut end = prefix.to_vec();
    end.push(0xFF);
    let rows = db
        .get_range(prefix.as_ref(), &end, limit.max(1))
        .await
        .map_err(|e| Status::internal(format!("snapshot scan failed: {}", e)))?;
    let mut out = Vec::new();
    for (key, value) in rows {
        if key.len() < 9 {
            continue;
        }
        let tail = &key[key.len() - 9..];
        let Ok(node_bytes) = <[u8; 9]>::try_from(tail) else {
            continue;
        };
        let node_id = NodeId::from_bytes(&node_bytes).to_string();
        let data: SnapshotNodeData = match decode_cbor(&value) {
            Ok(data) => data,
            Err(_) => continue,
        };
        out.push(SnapshotNode {
            entity_type: entity_type.to_string(),
            node_id,
            data,
        });
    }
    Ok(out)
}
