use crate::cdc::CdcAccumulator;
use pelago_core::PelagoError;

pub fn cdc_for_site(site_id: &str) -> CdcAccumulator {
    CdcAccumulator::new(site_id)
}

pub async fn commit_or_internal(
    trx: foundationdb::Transaction,
    action: &str,
) -> Result<(), PelagoError> {
    trx.commit()
        .await
        .map_err(|e| PelagoError::Internal(format!("Failed to {}: {}", action, e)))?;
    Ok(())
}
