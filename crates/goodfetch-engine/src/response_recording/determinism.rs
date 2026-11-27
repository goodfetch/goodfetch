//! Deterministic time and randomness for replayed browser pages (ADRs 0059,
//! 0073 and 0105; issue 801).
//!
//! Before a replayed page's first script runs, a single init script registered
//! once per page installs:
//! - `Date`, starting at the document's recorded fetch time (`WARC-Date`, or
//!   a test document's HTTP `Date` header) and running forward monotonically;
//! - `Math.random`, drawing from a deterministic `SplitMix64` generator seeded
//!   by a pure function of the recording's `warcinfo` record ID and the page's
//!   canonical URL (without its `#fragment`).
//!
//! Hosts mapped to loopback by `network.resolve` are reached live and are left
//! untouched by the init script.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// `SplitMix64` additive Weyl constant.
const SPLITMIX_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;

/// `SplitMix64` avalanche multipliers.
const SPLITMIX_MUL_1: u64 = 0xbf58_476d_1ce4_e5b9;
const SPLITMIX_MUL_2: u64 = 0x94d0_49bb_1331_11eb;

/// `2^53`, used to turn the top 53 bits of a 64-bit word into a uniform
/// IEEE-754 double in `[0.0, 1.0)`.
const TWO_POW_53: f64 = 9_007_199_254_740_992.0;

/// Strips any `#fragment` from `url` so seeding and document-time lookup match
/// WARC response matching.
pub(crate) fn canonical_url(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

/// Mixes `z` through `SplitMix64`'s finalizer.
const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(SPLITMIX_MUL_1);
    z = (z ^ (z >> 27)).wrapping_mul(SPLITMIX_MUL_2);
    z ^ (z >> 31)
}

/// Pure 64-bit seed derived from a recording's `warcinfo` record ID and a
/// page's URL (without its `#fragment`).
///
/// The exact same UTF-8 FNV-1a-64 + `SplitMix64` finalizer runs in the browser
/// init script using `TextEncoder` and `BigInt`.
pub fn seed_of(warcinfo_id: &str, url: &str) -> u64 {
    let mut hash = FNV_OFFSET;
    for &byte in warcinfo_id.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash ^= 0xff;
    hash = hash.wrapping_mul(FNV_PRIME);
    for &byte in canonical_url(url).as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    mix64(hash)
}

/// Deterministic `Math.random` generator seeded by `(warcinfo_id, url)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeededRandom {
    state: u64,
}

impl SeededRandom {
    /// Creates the generator for `warcinfo_id` and `url`.
    pub fn new(warcinfo_id: &str, url: &str) -> Self {
        Self {
            state: seed_of(warcinfo_id, url),
        }
    }

    /// Draws the next uniform double in `[0.0, 1.0)`.
    pub fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(SPLITMIX_INCREMENT);
        let z = mix64(self.state);
        #[expect(
            clippy::cast_precision_loss,
            reason = "z >> 11 fits in 53 bits, which f64 represents exactly"
        )]
        let mantissa = (z >> 11) as f64;
        mantissa / TWO_POW_53
    }
}

/// Parses the first valid HTTP `Date` header in `headers` (IMF-fixdate,
/// RFC 850 or asctime), independent of host locale and time zone.
pub fn http_date_from_headers(headers: &[(String, String)]) -> Option<SystemTime> {
    headers.iter().find_map(|(name, value)| {
        name.eq_ignore_ascii_case("date")
            .then(|| httpdate::parse_http_date(value.trim()).ok())
            .flatten()
    })
}

/// Converts `time` to Unix epoch milliseconds (signed, supporting dates before
/// 1970 as well as after), independent of host time zone and locale.
pub fn system_time_millis(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => {
            let millis = i64::try_from(before.duration().as_millis()).ok()?;
            millis.checked_neg()
        }
    }
}

/// Builds the one init script registered once per page for a replaying Run
/// (ADR 0105): installs `Date` starting at the document's recorded fetch time
/// (or a routed test document's `Date` header) and running forward, and seeds
/// `Math.random` from `warcinfo_id` and the page's canonical URL.
///
/// Hosts in `live_hosts` (those `network.resolve` maps to loopback) are left
/// live with native `Date` and `Math.random`.
pub fn init_script(
    warcinfo_id: &str,
    document_times: &BTreeMap<String, i64>,
    live_hosts: &[String],
) -> String {
    let warcinfo_json =
        serde_json::to_string(warcinfo_id).expect("invariant: string serializes as JSON");
    let times_json = serde_json::to_string(document_times)
        .expect("invariant: BTreeMap<String, i64> serializes as JSON");
    let live_json =
        serde_json::to_string(live_hosts).expect("invariant: Vec<String> serializes as JSON");
    format!(
        r##"(() => {{
  const warcinfoId = {warcinfo_json};
  const recordedTimes = {times_json};
  const liveHosts = new Set({live_json});
  const href = (globalThis.location && typeof globalThis.location.href === "string")
    ? globalThis.location.href
    : "";
  if (!href || href === "about:blank") return;
  let hostname = "";
  try {{
    hostname = new URL(href).hostname.toLowerCase().replace(/\.$/, "");
  }} catch {{
    return;
  }}
  if (liveHosts.has(hostname)) return;
  const hashIndex = href.indexOf("#");
  const canonicalUrl = hashIndex >= 0 ? href.slice(0, hashIndex) : href;

  const MASK = 0xffffffffffffffffn;
  const FNV_OFFSET = 0xcbf29ce484222325n;
  const FNV_PRIME = 0x100000001b3n;
  const INC = 0x9e3779b97f4a7c15n;
  const MUL1 = 0xbf58476d1ce4e5b9n;
  const MUL2 = 0x94d049bb133111ebn;
  const mix64 = (z) => {{
    z = ((z ^ (z >> 30n)) * MUL1) & MASK;
    z = ((z ^ (z >> 27n)) * MUL2) & MASK;
    return (z ^ (z >> 31n)) & MASK;
  }};
  const enc = new TextEncoder();
  let h = FNV_OFFSET;
  for (const b of enc.encode(warcinfoId)) {{
    h = ((h ^ BigInt(b)) * FNV_PRIME) & MASK;
  }}
  h = ((h ^ 0xffn) * FNV_PRIME) & MASK;
  for (const b of enc.encode(canonicalUrl)) {{
    h = ((h ^ BigInt(b)) * FNV_PRIME) & MASK;
  }}
  let state = mix64(h);
  Math.random = function random() {{
    state = (state + INC) & MASK;
    const z = mix64(state);
    return Number(z >> 11n) / 9007199254740992;
  }};

  const routed = globalThis.__gfReplayDates;
  const recordedAtMs = (routed && Object.prototype.hasOwnProperty.call(routed, canonicalUrl))
    ? routed[canonicalUrl]
    : (Object.prototype.hasOwnProperty.call(recordedTimes, canonicalUrl)
      ? recordedTimes[canonicalUrl]
      : null);
  if (typeof recordedAtMs !== "number" || !Number.isFinite(recordedAtMs)) return;

  const RealDate = globalThis.Date;
  const startPerf = performance.now();
  const nowMs = () => Math.trunc(recordedAtMs + Math.max(0, performance.now() - startPerf));
  function ReplayedDate(...args) {{
    if (!new.target) {{
      return new RealDate(nowMs()).toString();
    }}
    if (args.length === 0) {{
      return Reflect.construct(RealDate, [nowMs()], new.target);
    }}
    return Reflect.construct(RealDate, args, new.target);
  }}
  ReplayedDate.prototype = RealDate.prototype;
  Object.setPrototypeOf(ReplayedDate, RealDate);
  ReplayedDate.now = nowMs;
  ReplayedDate.parse = RealDate.parse.bind(RealDate);
  ReplayedDate.UTC = RealDate.UTC.bind(RealDate);
  globalThis.Date = ReplayedDate;
}})();"##
    )
}

/// Tiny init-script snippet registering a test document's HTTP `Date` header
/// when a fulfilling route answers a main-frame navigation on a replaying Run.
pub fn routed_date_script(url: &str, millis: i64) -> String {
    let url_json =
        serde_json::to_string(canonical_url(url)).expect("invariant: string serializes as JSON");
    format!(
        "(() => {{ const d = globalThis.__gfReplayDates || (globalThis.__gfReplayDates = Object.create(null)); d[{url_json}] = {millis}; }})();"
    )
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn seeded_generator_is_a_pure_function_of_record_id_and_url(
            id in "\\PC{0,64}",
            url in "https://[a-z0-9.-]{1,24}/[a-z0-9/_?-]{0,40}",
            fragment_a in "[a-z0-9_-]{0,16}",
            fragment_b in "[a-z0-9_-]{0,16}",
        ) {
            let with_a = format!("{url}#{fragment_a}");
            let with_b = format!("{url}#{fragment_b}");
            prop_assert_eq!(seed_of(&id, &url), seed_of(&id, &url));
            prop_assert_eq!(seed_of(&id, &with_a), seed_of(&id, &with_b));

            let mut first = SeededRandom::new(&id, &url);
            let mut second = SeededRandom::new(&id, &with_a);
            for _ in 0..16 {
                let a = first.next_f64();
                let b = second.next_f64();
                prop_assert_eq!(a.to_bits(), b.to_bits());
                prop_assert!(a.is_finite());
                prop_assert!(a >= 0.0);
                prop_assert!(a < 1.0);
            }
        }

        #[test]
        fn distinct_ids_or_urls_yield_distinct_seeds(
            id_a in "[a-z0-9:<>/._-]{1,48}",
            id_b in "[a-z0-9:<>/._-]{1,48}",
            url_a in "https://[a-z0-9]{1,12}\\.test/[a-z0-9]{0,16}",
            url_b in "https://[a-z0-9]{1,12}\\.test/[a-z0-9]{0,16}",
        ) {
            prop_assume!(id_a != id_b || url_a != url_b);
            prop_assert_ne!(seed_of(&id_a, &url_a), seed_of(&id_b, &url_b));
        }
    }

    #[test]
    fn http_date_header_parses_to_utc_millis() {
        let headers = vec![(
            "Date".to_owned(),
            "Wed, 15 Jun 2022 08:30:00 GMT".to_owned(),
        )];
        let parsed = http_date_from_headers(&headers).expect("valid HTTP Date");
        assert_eq!(system_time_millis(parsed), Some(1_655_281_800_000));
    }
}
