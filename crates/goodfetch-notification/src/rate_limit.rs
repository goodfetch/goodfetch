//! The driver's rate limit (ADR 0027): at most one send a second to each
//! destination, after a burst of five, every sender spaced in the order it
//! asked. Each destination keeps the moment its next send would be due at
//! the steady rate (the generic cell rate algorithm), so concurrent events
//! each take their own turn.

use std::collections::HashMap;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use tokio::sync::Mutex;
use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// The steady spacing of sends to one destination.
pub const INTERVAL: Duration = Duration::from_secs(1);

/// How many sends to one destination may go at once after a quiet spell.
pub const BURST: u32 = 5;

/// The turns of every destination the driver sends to.
#[derive(Debug)]
pub struct RateLimit {
    clock: Clock,
    /// For each destination, when its next send is due at the steady rate.
    due: Mutex<HashMap<String, Instant>>,
}

impl RateLimit {
    /// A rate limit on `clock`, every destination quiet.
    pub fn new(clock: Clock) -> Self {
        Self {
            clock,
            due: Mutex::new(HashMap::new()),
        }
    }

    /// Waits for this sender's turn to send to `destination`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the turn it took, which stays taken:
    /// the senders after it wait as if it had sent.
    pub async fn wait_turn(&self, destination: &str) {
        let turn = {
            let mut due = self.due.lock().await;
            let now = self.clock.now();
            let next = if let Some(due) = due.get_mut(destination) {
                let next = (*due).max(now);
                *due = next + INTERVAL;
                next
            } else {
                due.insert(destination.to_owned(), now + INTERVAL);
                now
            };
            next.checked_sub(INTERVAL * (BURST - 1))
                .map_or(now, |turn| turn.max(now))
        };
        self.clock.sleep_until(turn).await;
    }
}
