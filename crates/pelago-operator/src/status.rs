use crate::api::{PelagoSiteStatus, StatusCondition};
use chrono::Utc;

pub const CONDITION_READY: &str = "Ready";
pub const CONDITION_CONFIG_VALID: &str = "ConfigValid";
pub const CONDITION_API_AVAILABLE: &str = "ApiAvailable";
pub const CONDITION_REPLICATOR_AVAILABLE: &str = "ReplicatorAvailable";
pub const CONDITION_PROGRESSING: &str = "Progressing";
pub const CONDITION_DEGRADED: &str = "Degraded";

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub fn set_condition(
    status: &mut PelagoSiteStatus,
    condition_type: &str,
    condition_status: &str,
    reason: impl Into<String>,
    message: impl Into<String>,
) {
    let now = now_rfc3339();
    let reason = Some(reason.into());
    let message = Some(message.into());

    if let Some(existing) = status
        .conditions
        .iter_mut()
        .find(|c| c.condition_type == condition_type)
    {
        let changed = existing.status != condition_status;
        existing.status = condition_status.to_string();
        existing.reason = reason;
        existing.message = message;
        if changed {
            existing.last_transition_time = now;
        }
        return;
    }

    status.conditions.push(StatusCondition {
        condition_type: condition_type.to_string(),
        status: condition_status.to_string(),
        reason,
        message,
        last_transition_time: now,
    });
}

pub fn clear_condition(status: &mut PelagoSiteStatus, condition_type: &str) {
    status
        .conditions
        .retain(|condition| condition.condition_type != condition_type);
}
