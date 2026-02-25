use super::{QueryFilter, WatchContext, WatchFilter};
use pelago_storage::{
    cdc_position_exists, oldest_cdc_position, upsert_query_watch_state, PelagoDb, Versionstamp,
};
use std::collections::HashSet;
use tonic::Status;

pub(super) async fn persist_query_watch_state(
    db: &PelagoDb,
    ctx: &WatchContext,
    filter: &WatchFilter,
    matched_nodes: &HashSet<String>,
) -> Result<(), pelago_core::PelagoError> {
    if !matches!(filter, WatchFilter::Query(_)) {
        return Ok(());
    }
    let Some(state_id) = ctx.query_state_id.as_deref() else {
        return Ok(());
    };
    upsert_query_watch_state(
        db,
        &ctx.database,
        &ctx.namespace,
        state_id,
        &ctx.after,
        matched_nodes,
    )
    .await
}

pub(super) fn should_restore_query_watch_state(ctx: &WatchContext, filter: &WatchFilter) -> bool {
    matches!(filter, WatchFilter::Query(_))
        && ctx.query_state_id.is_some()
        && !ctx.include_initial_snapshot
        && !ctx.after.is_zero()
}

pub(super) fn parse_resume_after(resume_after: &[u8]) -> Result<Option<Versionstamp>, Status> {
    if resume_after.is_empty() {
        return Ok(None);
    }
    let vs = Versionstamp::from_bytes(resume_after)
        .ok_or_else(|| Status::invalid_argument("resume_after must be a 10-byte versionstamp"))?;
    Ok(Some(vs))
}

pub(super) fn resolve_resume_position(
    requested: Versionstamp,
    exists: bool,
    oldest_available: Option<Versionstamp>,
) -> Result<Versionstamp, Status> {
    if exists {
        return Ok(requested);
    }
    if let Some(oldest) = oldest_available {
        if requested < oldest {
            return Err(Status::out_of_range(
                "resume_after versionstamp is expired relative to CDC retention",
            ));
        }
    }
    Ok(Versionstamp::zero())
}

pub(super) async fn validate_resume_position(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    resume_after: &[u8],
) -> Result<Versionstamp, Status> {
    let Some(vs) = parse_resume_after(resume_after)? else {
        return Ok(Versionstamp::zero());
    };

    let exists = cdc_position_exists(db, database, namespace, &vs)
        .await
        .map_err(|e| Status::internal(format!("resume validation failed: {}", e)))?;
    if exists {
        return Ok(vs);
    }
    let oldest = oldest_cdc_position(db, database, namespace)
        .await
        .map_err(|e| Status::internal(format!("resume oldest lookup failed: {}", e)))?;
    resolve_resume_position(vs, false, oldest)
}

pub(super) fn node_key(entity_type: &str, node_id: &str) -> String {
    format!("{}:{}", entity_type, node_id)
}

pub(super) fn compute_query_state_id(
    database: &str,
    namespace: &str,
    filter: &QueryFilter,
) -> String {
    let mut signature = format!(
        "{}\n{}\n{}\n{}",
        database,
        namespace,
        filter.entity_type,
        filter.cel_expression.clone().unwrap_or_default()
    );

    if let Some(pql) = &filter.pql_predicate {
        signature.push('\n');
        signature.push_str(pql.root_entity_type.as_deref().unwrap_or_default());
        signature.push('\n');
        signature.push_str(pql.cel_expression.as_deref().unwrap_or_default());
    }

    format!("qws-{:016x}", fnv1a64(signature.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET_BASIS;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}
