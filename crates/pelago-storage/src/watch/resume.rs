//! Resume position validation
//!
//! Validates that a client's resume position is still within the CDC
//! retention window. If the position has been garbage-collected, the
//! client must either restart from "now" or accept the oldest available.

use crate::cdc::Versionstamp;
use crate::db::PelagoDb;
use crate::Subspace;
use pelago_core::PelagoError;

/// Result of validating a resume position against the CDC log.
pub enum ResumeStatus {
    /// Position is within the CDC log — safe to resume from here.
    Valid,
    /// Position is before the oldest retained CDC entry.
    Expired { oldest_available: Versionstamp },
    /// Position is zero or in the future — start from current head.
    StartFromNow,
}

/// Validate that a resume position exists within the CDC log's retention window.
///
/// Returns `Valid` if the position can be used as a resume point,
/// `Expired` if the position has been garbage-collected, or
/// `StartFromNow` if the position is empty/zero.
pub async fn validate_resume_position(
    db: &PelagoDb,
    database: &str,
    namespace: &str,
    position: &Versionstamp,
) -> Result<ResumeStatus, PelagoError> {
    if position.is_zero() {
        return Ok(ResumeStatus::StartFromNow);
    }

    // Try to read the CDC entry at this position
    let subspace = Subspace::namespace(database, namespace).cdc();
    let prefix = subspace.prefix();

    // Build key for the exact position
    let mut exact_key = prefix.to_vec();
    exact_key.extend_from_slice(position.to_bytes());

    // Check if this exact entry exists
    if db.get(&exact_key).await?.is_some() {
        return Ok(ResumeStatus::Valid);
    }

    // Position doesn't exist — check if it's before the oldest entry
    let range_end = subspace.range_end().to_vec();
    let oldest = db.get_range(prefix, &range_end, 1).await?;

    if let Some((oldest_key, _)) = oldest.first() {
        let prefix_len = prefix.len();
        if oldest_key.len() >= prefix_len + 10 {
            let vs_bytes = &oldest_key[prefix_len..prefix_len + 10];
            if let Some(oldest_vs) = Versionstamp::from_bytes(vs_bytes) {
                if position < &oldest_vs {
                    return Ok(ResumeStatus::Expired {
                        oldest_available: oldest_vs,
                    });
                }
            }
        }
    }

    // Position is in the future or CDC log is empty
    Ok(ResumeStatus::StartFromNow)
}
