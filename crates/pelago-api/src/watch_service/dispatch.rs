use pelago_proto::WatchEvent;
use tokio::sync::mpsc;
use tonic::Status;

#[derive(Debug)]
pub(super) enum DispatchResult {
    Sent,
    Backpressured,
    DropSubscription(Status),
    ConsumerClosed,
}

pub(super) fn try_dispatch_watch_event(
    tx: &mpsc::Sender<Result<WatchEvent, Status>>,
    event: WatchEvent,
    dropped_events: &mut u32,
    max_dropped_events: u32,
) -> DispatchResult {
    match tx.try_send(Ok(event)) {
        Ok(_) => {
            *dropped_events = 0;
            DispatchResult::Sent
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            *dropped_events = dropped_events.saturating_add(1);
            if *dropped_events >= max_dropped_events {
                DispatchResult::DropSubscription(Status::resource_exhausted(
                    "watch consumer is too slow; subscription dropped",
                ))
            } else {
                DispatchResult::Backpressured
            }
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => DispatchResult::ConsumerClosed,
    }
}
