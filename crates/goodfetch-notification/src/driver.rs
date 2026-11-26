//! The Notification channel driver (ADR 0129): what every channel shares.
//! A channel fills [`Channel::send`] and its translation point,
//! [`Channel::map_failure`]; the driver waits for the destination's turn,
//! claims the attempt in the delivery ledger, sends, and on a transient
//! failure sends again on the Standard Webhooks schedule, honouring the
//! destination's `Retry-After`, until the attempts run out. A permanent
//! failure fails the delivery at once. Each failure is logged at `warn`,
//! with its name and never the party's text.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::run_state::RunStateError;

use crate::failure::Failure;
use crate::ledger::{Claim, Entry, Ledger, Settled};
use crate::message::Message;
use crate::rate_limit::RateLimit;

#[cfg(test)]
mod tests;

/// The wait before each attempt, after the previous one failed: the
/// Standard Webhooks schedule. The first attempt goes at once; after the
/// last, the delivery fails.
pub const SCHEDULE: [Duration; 10] = [
    Duration::ZERO,
    Duration::from_secs(5),
    Duration::from_mins(5),
    Duration::from_mins(30),
    Duration::from_hours(2),
    Duration::from_hours(5),
    Duration::from_hours(10),
    Duration::from_hours(14),
    Duration::from_hours(20),
    Duration::from_hours(24),
];

/// The longest `Retry-After` the driver honours: a longer one waits this.
pub const RETRY_AFTER_CAP: Duration = Duration::from_hours(24);

/// One configured Notification channel, such as a `webhook`.
pub trait Channel: Send + Sync {
    /// Why its party did not take a message, before translation.
    type Failure: Send;

    /// Its name in configuration, the delivery ledger's key with the event.
    fn name(&self) -> &str;

    /// Where it sends, the rate limit's key, such as the webhook's host.
    fn destination(&self) -> &str;

    /// Sends `message` once.
    ///
    /// # Errors
    ///
    /// Its party's failure, which [`Self::map_failure`] translates.
    fn send(&self, message: &Message) -> impl Future<Output = Result<(), Self::Failure>> + Send;

    /// The translation point (ADR 0127): `failure` in the failure enum,
    /// [`Failure::Unrecognized`] for one it does not know.
    fn map_failure(&self, failure: Self::Failure) -> Failure;
}

/// The driver of one Run's Notifications.
#[derive(Clone, Debug)]
pub struct Driver {
    ledger: Ledger,
    rate_limit: Arc<RateLimit>,
    clock: Clock,
}

impl Driver {
    /// A driver writing `ledger`, the Run's, spacing sends by `rate_limit`,
    /// which every Run of the engine shares.
    pub fn new(ledger: Ledger, rate_limit: Arc<RateLimit>, clock: Clock) -> Self {
        Self {
            ledger,
            rate_limit,
            clock,
        }
    }

    /// Delivers `message` on `channel`, or carries on its delivery after a
    /// resume, and answers what the ledger then holds: delivered, failed,
    /// uncertain (in flight when an earlier engine stopped), or sending,
    /// when this engine is already delivering it.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read or written: nothing is sent past a
    /// claim that was not committed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future while it waits leaves the ledger as it was: a
    /// resumed Run carries on. Dropping it while it sends leaves the attempt
    /// claimed, settled `uncertain` by the next run epoch.
    pub async fn deliver<C: Channel>(
        &self,
        channel: &C,
        message: &Message,
    ) -> Result<Entry, RunStateError> {
        let (event, name) = (&message.event_id, channel.name());
        loop {
            let attempt = match self.ledger.entry(event, name).await? {
                None => 0,
                Some(Entry::Waiting { attempt, at }) => {
                    let wait = at.duration_since(self.clock.wall()).unwrap_or_default();
                    self.clock.sleep(wait).await;
                    attempt
                }
                Some(Entry::Sending { attempt, .. }) => attempt,
                Some(settled) => return Ok(settled),
            };
            self.rate_limit.wait_turn(channel.destination()).await;
            if let Claim::Held(entry) = self.ledger.claim(event, name, attempt).await? {
                return Ok(entry);
            }
            let failure = match channel.send(message).await {
                Ok(()) => {
                    self.ledger.settle(event, name, Settled::Delivered).await?;
                    return Ok(Entry::Delivered { attempt });
                }
                Err(failure) => channel.map_failure(failure),
            };
            tracing::warn!(
                channel = name,
                event = event.as_str(),
                attempt,
                failure = failure.name(),
                "a Notification was not delivered"
            );
            let next = attempt + 1;
            let wait = usize::try_from(next)
                .ok()
                .and_then(|next| SCHEDULE.get(next));
            match wait {
                Some(wait) if failure.is_transient() => {
                    let asked = failure.retry_after().unwrap_or_default();
                    let at = self
                        .clock
                        .point_in_time_after((*wait).max(asked.min(RETRY_AFTER_CAP)));
                    let retry = Settled::RetryAt { attempt: next, at };
                    self.ledger.settle(event, name, retry).await?;
                }
                _ => {
                    let failed = Settled::Failed(failure.name());
                    self.ledger.settle(event, name, failed).await?;
                    return Ok(Entry::Failed {
                        attempt,
                        failure: failure.name().to_owned(),
                    });
                }
            }
        }
    }
}
