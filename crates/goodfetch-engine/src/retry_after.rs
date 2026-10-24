//! A site's `Retry-After` (ADR 0118; RFC 9110 §10.2.3), read in both its
//! forms: a number of seconds, and an HTTP-date in each of RFC 9110
//! §5.6.7's three formats. A date is measured against the response's own
//! `Date` when it has one that parses, and against the engine's clock
//! otherwise, so a clock skewed on either side changes nothing. A fetch
//! tier reads it on a 429, 502, 503 or 504, and on a 3xx; a crawl's domain
//! waits for it in Politeness, and a Collie page reports it on the call
//! that met it. A pure function of the reading it is given (ADR 0059).

use std::time::{Duration, SystemTime};

/// What a `Retry-After` asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryAfter {
    /// No request before this moment, of the engine's clock.
    Until(SystemTime),
    /// No wait: `0`, or a date not later than the response's own.
    NoWait,
    /// Neither form: it adds no wait, and the Run trace notes it.
    Unparsable,
}

/// What the `Retry-After` `value` of a response received at `received`,
/// with the `Date` header `date` if it has one, asks.
pub fn read(value: &str, date: Option<&str>, received: SystemTime) -> RetryAfter {
    let value = value.trim_matches([' ', '\t']);
    let wait = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Duration::from_secs(value.parse().unwrap_or(u64::MAX))
    } else {
        let Ok(asked) = httpdate::parse_http_date(value) else {
            return RetryAfter::Unparsable;
        };
        let since = date
            .and_then(|date| httpdate::parse_http_date(date.trim_matches([' ', '\t'])).ok())
            .unwrap_or(received);
        asked.duration_since(since).unwrap_or_default()
    };
    if wait.is_zero() {
        return RetryAfter::NoWait;
    }
    RetryAfter::Until(
        received
            .checked_add(wait)
            .unwrap_or_else(|| far_future(received)),
    )
}

/// The time a site asked for, as an error's `retry_after` detail writes it
/// (ADRs 0092 and 0118): RFC 3339 in UTC, to the millisecond when it has
/// a fraction.
pub fn detail(at: SystemTime) -> String {
    jiff::Timestamp::try_from(at).map_or_else(
        |_| jiff::Timestamp::MAX.to_string(),
        |at| at.round(jiff::Unit::Millisecond).unwrap_or(at).to_string(),
    )
}

/// The time an error's `retry_after` detail names, when it names one.
pub fn of_detail(detail: &serde_json::Value) -> Option<SystemTime> {
    let at: jiff::Timestamp = detail.as_str()?.parse().ok()?;
    Some(SystemTime::from(at))
}

/// The latest moment after `from` the platform's clock holds, for a wait
/// too long to add.
fn far_future(from: SystemTime) -> SystemTime {
    let mut wait = Duration::from_secs(u64::MAX);
    loop {
        if let Some(moment) = from.checked_add(wait) {
            return moment;
        }
        wait /= 2;
    }
}

#[cfg(test)]
mod tests;
