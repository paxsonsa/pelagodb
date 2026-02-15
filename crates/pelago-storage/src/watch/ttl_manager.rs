//! TTL Manager
//!
//! Periodically scans subscriptions for expiry, sends a SUBSCRIPTION_ENDED
//! event to the client, and cleans up the subscription.

use std::sync::Arc;
use std::time::Duration;

use crate::watch::registry::SubscriptionRegistry;
use pelago_proto::{SubscriptionEndReason, WatchEvent, WatchEventType};

/// Background task that checks for and expires TTL-exceeded subscriptions.
pub struct TtlManager {
    registry: Arc<SubscriptionRegistry>,
}

impl TtlManager {
    pub fn new(registry: Arc<SubscriptionRegistry>) -> Self {
        Self { registry }
    }

    /// Run the TTL expiry loop until shutdown.
    pub async fn run(&self, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    tracing::info!("TtlManager shutting down");
                    break;
                }
                _ = interval.tick() => {
                    self.expire_subscriptions().await;
                }
            }
        }
    }

    async fn expire_subscriptions(&self) {
        let expired = self.registry.get_expired_subscriptions().await;

        for sub_id in expired {
            // Send termination event to client
            if let Some(sender) = self.registry.get_sender(&sub_id).await {
                let event = WatchEvent {
                    event_type: WatchEventType::SubscriptionEnded as i32,
                    end_reason: SubscriptionEndReason::TtlExpired as i32,
                    subscription_id: sub_id.clone(),
                    ..Default::default()
                };
                let _ = sender.try_send(event);
            }

            // Clean up the subscription
            let _ = self.registry.cancel_subscription(&sub_id).await;
            tracing::debug!(subscription_id = %sub_id, "Subscription expired (TTL)");
        }
    }
}
