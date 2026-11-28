//! Engine-emitted Notifications (ADRs 0027, 0041 and 0087): turns a Usage
//! ledger threshold crossing into an [`EngineNotification`], delivers it
//! through the composition's [`NotificationDispatcher`], and marks it sent in
//! the ledger once delivered.

use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::notifications::NotificationEvent;

use crate::engine_context::EngineContext;
pub use crate::engine_context::{EngineNotification, NotificationDispatcher};
use crate::ledger::{ThresholdCrossing, point_in_time};

/// Builds the `limit_threshold` [`EngineNotification`] for `crossing` on
/// `product`.
#[must_use]
pub fn threshold_crossing_notification(
    crossing: &ThresholdCrossing,
    product: Product,
) -> EngineNotification {
    let event_id = threshold_event_id(crossing);
    let scope = if crossing.limit.scope.id.is_empty() {
        crossing.limit.scope.kind.clone()
    } else {
        format!(
            "{} ({})",
            crossing.limit.scope.kind, crossing.limit.scope.id
        )
    };
    let unit = crossing
        .limit
        .currency
        .as_deref()
        .map_or_else(String::new, |currency| format!(" ({currency})"));
    let period = crossing
        .limit
        .known_period()
        .map_or_else(String::new, |period| {
            format!(" per {} ({})", period.every.as_str(), period.time_zone)
        });
    let title = format!(
        "Limit reached {} % ({} {})",
        crossing.threshold, scope, crossing.limit.usage_kind
    );
    let text = format!(
        "The {scope} Limit on {}{unit}{period} ({}) reached {} %.",
        crossing.limit.usage_kind, crossing.limit_amount, crossing.threshold
    );
    EngineNotification {
        event_id,
        event: NotificationEvent::LIMIT_THRESHOLD,
        run_id: None,
        schedule_id: None,
        product,
        title,
        text,
        link: None,
        occurred_at: point_in_time(crossing.crossed_at),
    }
}

/// Deterministic, Standard Webhooks-safe event ID for one threshold crossing:
/// 1 to 128 ASCII letters, digits, `_` or `-` (never `.`, which would make a
/// webhook's signed text ambiguous).
fn threshold_event_id(crossing: &ThresholdCrossing) -> String {
    let period = if crossing.period_start.is_empty() {
        "none".to_owned()
    } else {
        crossing
            .period_start
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect()
    };
    let id = format!(
        "limit_threshold_{}_{}_{}_{}",
        crossing.limit.id, period, crossing.limit_amount, crossing.threshold
    );
    if id.len() <= 128 {
        id
    } else {
        id[..128].to_owned()
    }
}

/// Delivers each threshold crossing in ascending threshold order through
/// `context`'s [`NotificationDispatcher`], marking each sent in the Usage
/// ledger only after delivery succeeds (ADR 0041).
pub(crate) async fn dispatch_threshold_crossings(
    context: &EngineContext,
    product: Product,
    mut crossings: Vec<ThresholdCrossing>,
) {
    if crossings.is_empty() {
        return;
    }
    crossings.sort_by_key(|crossing| (crossing.limit.id, crossing.threshold));
    for crossing in crossings {
        let notification = threshold_crossing_notification(&crossing, product);
        let delivered = context.dispatch_notification(notification).await;
        if !delivered {
            tracing::warn!(
                limit = crossing.limit.id,
                threshold = crossing.threshold,
                "a Limit threshold Notification was not delivered; it remains unsent in the Usage ledger"
            );
            continue;
        }
        let ledger = context.ledger().clone();
        let now = context.now();
        let owner = Arc::clone(context.owner());
        if let Err(error) = owner
            .blocking(move || ledger.mark_threshold_sent(&crossing, now))
            .await
        {
            tracing::warn!(
                %error,
                "a delivered Limit threshold Notification could not be marked sent in the Usage ledger"
            );
        }
    }
}
