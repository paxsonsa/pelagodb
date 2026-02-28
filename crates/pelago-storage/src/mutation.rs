use crate::cdc::CdcAccumulator;
use crate::failpoints;
use pelago_core::PelagoError;

pub fn cdc_for_site(site_id: &str) -> CdcAccumulator {
    CdcAccumulator::new(site_id)
}

pub async fn commit_or_internal(
    trx: foundationdb::Transaction,
    action: &str,
) -> Result<(), PelagoError> {
    failpoints::inject("tx.commit.before")?;
    failpoints::inject_action("tx.commit.before_action", action)?;
    trx.commit()
        .await
        .map_err(|e| PelagoError::Internal(format!("Failed to {}: {}", action, e)))?;
    Ok(())
}
