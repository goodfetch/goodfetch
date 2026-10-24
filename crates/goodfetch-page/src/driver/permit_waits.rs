//! A page's waits for its navigation gate's permits, which count against
//! no timeout (ADRs 0080 and 0093): while any is under way, the deadline of
//! each navigation or `page.request` of the page moves on by as long.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use tokio::sync::watch;
use tokio::time::Instant;

/// The waits of one page, on its Run's clock.
pub(crate) struct PermitWaits {
    clock: Clock,
    state: watch::Sender<Waits>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Waits {
    /// How many are under way.
    pending: u32,
    /// Since when one has been, while any is.
    since: Option<Instant>,
    /// How long one was under way, in all, before `since`.
    waited: Duration,
}

/// One wait under way, which ends when dropped.
pub(crate) struct Waiting(Arc<PermitWaits>);

impl Drop for Waiting {
    fn drop(&mut self) {
        let now = self.0.clock.now();
        self.0.state.send_modify(|waits| {
            waits.pending = waits.pending.saturating_sub(1);
            if waits.pending == 0
                && let Some(since) = waits.since.take()
            {
                waits.waited += now.saturating_duration_since(since);
            }
        });
    }
}

/// A deadline that passed.
#[derive(Debug)]
pub(crate) struct Elapsed;

impl PermitWaits {
    /// The waits of a page whose Run reads time from `clock`.
    pub(crate) fn new(clock: Clock) -> Arc<Self> {
        Arc::new(Self {
            clock,
            state: watch::Sender::new(Waits::default()),
        })
    }

    /// Starts a wait, under way until the answer drops.
    pub(crate) fn begin(self: &Arc<Self>) -> Waiting {
        let now = self.clock.now();
        self.state.send_modify(|waits| {
            waits.pending += 1;
            waits.since.get_or_insert(now);
        });
        Waiting(Arc::clone(self))
    }

    /// How long a wait was under way, in all, until `now`.
    fn waited(&self, now: Instant) -> Duration {
        let waits = *self.state.borrow();
        waits.waited
            + waits
                .since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }

    /// Runs `future` within `limit`, the time waits are under way not
    /// counted.
    ///
    /// # Errors
    ///
    /// [`Elapsed`] once `limit` passed outside the waits.
    pub(crate) async fn within<F: Future>(
        &self,
        limit: Duration,
        future: F,
    ) -> Result<F::Output, Elapsed> {
        let start = self.clock.now();
        let waited_before = self.waited(start);
        let mut future = std::pin::pin!(future);
        let mut changes = self.state.subscribe();
        loop {
            changes.borrow_and_update();
            let now = self.clock.now();
            let deadline = start + limit + self.waited(now).saturating_sub(waited_before);
            // While a wait is under way the deadline moves with the clock:
            // only its end, or another wait's start, moves it again.
            let waiting = self.state.borrow().pending > 0;
            if !waiting && now >= deadline {
                return Err(Elapsed);
            }
            tokio::select! {
                output = &mut future => return Ok(output),
                () = self.clock.sleep_until(deadline), if !waiting => {}
                _ = changes.changed() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
