//! A crawl's pace on a domain key (spec #582's Settings): its delay
//! between starts, which a Crawl-delay and the adaptive delay lengthen, and
//! its rate, a token bucket every sharing run's starts draw on; its
//! admissions wait for both, and its crawl requests' own sends wait for
//! both again.

use std::time::Duration;

use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::foxhound::settings::{
    POLITENESS_BACKOFF_MAX, POLITENESS_DOMAIN_ADAPTIVE_DELAY, POLITENESS_DOMAIN_BURST,
    POLITENESS_DOMAIN_DELAY, POLITENESS_DOMAIN_RATE, SITE_PREFERENCES_MAX_CRAWL_DELAY,
    SITE_PREFERENCES_RESPECT_ROBOTS_TXT,
};

use super::{Limits, Pacing, duration_milliseconds};

/// A crawl's pacing of its starts on a domain key under its own settings:
/// its delay between starts and its rate, which its admissions wait for
/// and its crawl requests' own sends wait for again
/// ([`started_at_send`](super::started_at_send)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pace {
    /// `politeness.domain.delay`; `None` for none.
    delay: Option<Duration>,
    /// `site_preferences.max_crawl_delay`; `None` while
    /// `site_preferences.respect_robots_txt` is off, which waits for no
    /// Crawl-delay.
    max_crawl_delay: Option<Duration>,
    /// `politeness.backoff.max`, which caps the latency's mean, while
    /// `politeness.domain.adaptive_delay` is on; `None` while it is off.
    adaptive_max: Option<Duration>,
    /// `politeness.domain.rate` and `politeness.domain.burst`.
    pub(super) rate: Option<Rate>,
}

impl Pace {
    /// The pace `settings` give a crawl.
    pub fn of(settings: &Resolved) -> Self {
        Self {
            delay: settings
                .duration(POLITENESS_DOMAIN_DELAY.name)
                .filter(|delay| !delay.is_zero()),
            max_crawl_delay: (settings.boolean(SITE_PREFERENCES_RESPECT_ROBOTS_TXT.name)
                != Some(false))
            .then(|| settings.duration(SITE_PREFERENCES_MAX_CRAWL_DELAY.name))
            .flatten(),
            adaptive_max: (settings.boolean(POLITENESS_DOMAIN_ADAPTIVE_DELAY.name) == Some(true))
                .then(|| {
                    settings
                        .duration(POLITENESS_BACKOFF_MAX.name)
                        .unwrap_or_default()
                }),
            rate: Rate::of(settings),
        }
    }

    /// Whether it may hold a start back: a delay, a Crawl-delay waited
    /// out, the adaptive delay or a rate.
    pub(crate) fn paces(&self) -> bool {
        self.delay.is_some()
            || self.max_crawl_delay.is_some_and(|max| !max.is_zero())
            || self.adaptive_max.is_some()
            || self.rate.is_some()
    }

    /// The delay between starts on a domain key whose hosts ask for
    /// `crawl_delays` (ADR 0086) and whose responses' latency averages
    /// `latency` milliseconds: the longest of `politeness.domain.delay`,
    /// each Crawl-delay up to `site_preferences.max_crawl_delay`, so a
    /// shorter one never shortens it and a longer one never raises it, and,
    /// with the adaptive delay on, that latency up to
    /// `politeness.backoff.max`; `None` for no wait.
    pub(super) fn between_starts(
        &self,
        crawl_delays: &[Duration],
        latency: Option<i64>,
    ) -> Option<Duration> {
        let honoured = self.max_crawl_delay.and_then(|max| {
            crawl_delays
                .iter()
                .copied()
                .filter(|asked| *asked <= max)
                .max()
        });
        let adaptive = self.adaptive_max.zip(latency).map(|(max, latency)| {
            Duration::from_millis(u64::try_from(latency).unwrap_or_default()).min(max)
        });
        self.delay
            .max(honoured)
            .max(adaptive)
            .filter(|delay| !delay.is_zero())
    }

    /// The moment a crawl request's own send at `now` waits for on a key
    /// whose state is `limits` and `pacing` and whose hosts ask for
    /// `crawl_delays`: its delay after the key's last send, as the key's
    /// latency now lengthens it, and its rate's token of the key's sends;
    /// `None` when it may go. `now` is the whole millisecond the send falls
    /// in, rounded down: the key's last send and theoretical send are kept
    /// rounded up, so the send then never goes sooner than either allows.
    pub(super) fn send_waits_until(
        &self,
        limits: &Limits,
        pacing: &Pacing,
        crawl_delays: &[Duration],
        now: i64,
    ) -> Option<i64> {
        let delayed = self
            .between_starts(crawl_delays, pacing.latency)
            .zip(limits.last_send)
            .map(|(delay, last_send)| last_send.saturating_add(duration_milliseconds(delay)))
            .filter(|next| *next > now);
        let rated = self
            .rate
            .and_then(|rate| rate.waits_until(pacing.theoretical_send, now));
        delayed.max(rated)
    }
}

/// A run's per-domain rate: a token bucket of `burst` tokens, one coming
/// back every `interval`, as GCRA keeps it, one theoretical start per key
/// for the admissions and one for the sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rate {
    /// A minute divided by `politeness.domain.rate`, rounded up to the
    /// millisecond so the rate is never exceeded.
    pub(super) interval: i64,
    /// `politeness.domain.burst`, at least 1.
    burst: u64,
}

impl Rate {
    /// The rate `settings` give a crawl; `None` without
    /// `politeness.domain.rate`, or with one of 0.
    fn of(settings: &Resolved) -> Option<Self> {
        Self::per_minute(
            settings.count(POLITENESS_DOMAIN_RATE.name)?,
            settings.count(POLITENESS_DOMAIN_BURST.name).unwrap_or(1),
        )
    }

    /// `rate` requests a minute in bursts of `burst`, a burst of 0 read as
    /// 1; `None` for a rate of 0.
    fn per_minute(rate: u64, burst: u64) -> Option<Self> {
        (rate > 0).then(|| Self {
            interval: i64::try_from(60_000_u64.div_ceil(rate)).unwrap_or(i64::MAX),
            burst: burst.max(1),
        })
    }

    /// The moment a start at `now` must wait for, when the bucket whose
    /// theoretical start is `theoretical` holds no token; `None` when one
    /// is there.
    pub(super) fn waits_until(&self, theoretical: Option<i64>, now: i64) -> Option<i64> {
        let tolerance = self
            .interval
            .saturating_mul(i64::try_from(self.burst - 1).unwrap_or(i64::MAX));
        theoretical
            .map(|theoretical| theoretical.saturating_sub(tolerance))
            .filter(|earliest| *earliest > now)
    }

    /// The theoretical start after a start at `now` took a token.
    pub(super) fn taken(&self, theoretical: Option<i64>, now: i64) -> i64 {
        theoretical
            .map_or(now, |theoretical| theoretical.max(now))
            .saturating_add(self.interval)
    }
}
