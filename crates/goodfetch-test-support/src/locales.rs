//! The shared table of locales and time zones (ADR 0073), committed once as
//! `test-support/fixtures/locales.json`, which the TypeScript and Python
//! packages read too, so Period Limits, Schedules and formatting take the
//! same zone cases in every language.

use jiff::civil::Date;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The committed table, as its file holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocaleTable {
    /// The BCP 47 tags of the shared locales.
    locales: Vec<String>,
    /// The shared time zones.
    time_zones: Vec<TimeZoneCase>,
}

/// One shared time zone, with the days its daylight saving time starts and
/// ends on when a test needs them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeZoneCase {
    /// The zone's IANA name, such as `Europe/Paris`.
    pub name: String,
    /// The day in 2026 whose local clock skips an hour (the gap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>", extend("format" = "date"))]
    pub dst_gap: Option<Date>,
    /// The day in 2026 whose local clock repeats an hour (the overlap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>", extend("format" = "date"))]
    pub dst_overlap: Option<Date>,
}

/// The committed table's text.
const COMMITTED: &str = include_str!("../../../test-support/fixtures/locales.json");

fn committed() -> LocaleTable {
    serde_json::from_str(COMMITTED)
        .expect("invariant: the committed locale table parses, which the Quick suite checks")
}

/// The shared locales' BCP 47 tags: `en-US`, `de-DE`, `fr-FR`, `tr-TR`,
/// `ar-EG`, `hi-IN` and `ja-JP`.
pub fn locales() -> Vec<String> {
    committed().locales
}

/// The shared time zones: `UTC`, `America/St_Johns`, `Asia/Kathmandu`,
/// `Pacific/Chatham`, `Australia/Lord_Howe`, and `Europe/Paris` and
/// `America/New_York` with their days of daylight saving time's gap and
/// overlap.
pub fn time_zones() -> Vec<TimeZoneCase> {
    committed().time_zones
}

#[cfg(test)]
mod tests;
