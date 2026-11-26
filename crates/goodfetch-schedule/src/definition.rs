//! A Schedule's definition as `schedule.create` records it (ADR 0036): its
//! ID, given or generated, and its explicit IANA time zone, given or the
//! host's, never "local". Its trigger is [`crate::trigger`]'s; its target is
//! recorded as given.

use std::time::{SystemTime, UNIX_EPOCH};

use goodfetch_protocol::schedule::ScheduleId;
use jiff::tz::TimeZone;

/// A Schedule ID for a definition that gives none: `s` and a lowercase
/// ULID of the wall time `wall` and 80 bits of `draw` (ADR 0059), so the
/// same seed makes the same ID.
pub(crate) fn generated_id(wall: SystemTime, mut draw: impl FnMut() -> u64) -> ScheduleId {
    let milliseconds = wall.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    });
    let random = (u128::from(draw()) << 64 | u128::from(draw())) & ((1_u128 << 80) - 1);
    let ulid = ulid::Ulid::from_parts(milliseconds, random);
    ScheduleId::parse(&format!("s{}", ulid.to_string().to_lowercase()))
        .expect("invariant: `s` and a lowercase ULID is a Schedule ID")
}

/// A time zone no IANA name of the zone database names.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` names no IANA time zone, such as `Europe/Paris`")]
pub(crate) struct ZoneUnknown(pub(crate) String);

/// The zone a Schedule stores: `given`, when the zone database knows it,
/// else the host's zone `host`.
///
/// # Errors
///
/// [`ZoneUnknown`] for a `given` name the zone database does not know.
pub(crate) fn time_zone(given: Option<&str>, host: &str) -> Result<String, ZoneUnknown> {
    let Some(given) = given else {
        return Ok(host.to_owned());
    };
    let zone = TimeZone::get(given).map_err(|_| ZoneUnknown(given.to_owned()))?;
    Ok(zone.iana_name().unwrap_or(given).to_owned())
}

#[cfg(test)]
mod tests;
