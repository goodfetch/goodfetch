//! Foxhound's fetch tiers (spec #579; ADRs 0020, 0087, 0093, 0118, 0119
//! and 0129): the tiers that fetch a crawl request's document without the
//! browser, and the page they hand its handler. The fetch driver
//! (`driver`) owns what every tier shares, the transport-failure enum and
//! what each of its variants leads to, the engine's timeouts and the
//! decoding of a body's content codings (`decoding`); each tier that sends
//! HTTP itself fills its one primitive, `send(hop)`, with a transport
//! (`transports`: reqwest in `standard` and `identified` Sessions, `wreq`
//! in HTTP-only `stealth` ones, presenting the Session's HTTP profile,
//! `http_profile`, and browser-fetch, from inside the Session's browser
//! context, in any Session that may move to the browser). The
//! `tiers` module is the crate's one composition value (ADR 0087):
//! [`FetchTiers`], which serves a crawl request on the tier `fetch.tier`
//! names and hands its handler a Collie page served from the document.
//! Its own crate under `crates/foxhound/`, since only Foxhound links it
//! and Collie's `cargo-deny` ban list keeps its HTTP transports out of
//! Collie's executable (STYLE.md, reason 2).

mod canonical;
mod chain;
mod charset;
// The tests of the pure modules a benchmark compiles in through `#[path]`
// sit beside them, declared here, so the benchmark builds without them.
#[cfg(test)]
#[path = "charset/tests.rs"]
mod charset_tests;
mod choice;
mod content_types;
mod decoding;
mod discovered_api;
mod document;
mod driver;
mod expected_page;
mod gate;
mod hedging;
mod http_profile;
mod light_browser;
mod redirects;
mod redundant;
mod refresh;
mod served_calls;
mod sniffing;
#[cfg(test)]
#[path = "sniffing/tests.rs"]
mod sniffing_tests;
mod tiers;
mod transports;
mod use_signals;

pub use chain::{Following, Gating, HopGate, Skip, Stop, follow};
#[doc(hidden)]
pub use charset::fuzz_prescan;
pub use choice::{Choosing, Forced, LabelSettings, SetTier, TierChoice};
pub use content_types::ContentTypes;
pub use discovered_api::{Call as CapturedCall, Capture, Plan as ReplayPlan, infer as infer_plan};
pub use document::{FieldElement, Matched, SelectorInvalid, check_selector};
pub use driver::{
    Body, Deadlines, Fetched, Hop, Reuse, Sending, Sent, Transport, TransportFailure, fetch, met,
};
pub use gate::{HandOver, RobotsTxtFetch, content_signals, crawl_delay, disallowing};
pub use goodfetch_page::page_value::PooledSession;
pub use http_profile::HttpProfile;
pub use light_browser::{
    LIGHTPANDA, Prepared as LightBrowserBuild, pins as light_browser_pins,
    prepare as prepare_light_browser,
};
pub use redundant::Requester;
pub use refresh::fuzz_refresh;
pub use served_calls::serves as tier_serves;
pub use sniffing::sniff;
pub use tiers::{
    Chosen, FetchTiers, FieldElements, HandedOver, HttpTier, Judged, LinksUnread, PAGE_CHECK_CALL,
    Race, RaceEnd, ServedLinks, Signed, XPATH_FIELD_CALL, record_race, record_tier,
    tier_provenance,
};
pub use transports::{
    AddressCheck, LocalSource, Reqwest, TransportConfig, TransportUnavailable, Wreq,
};
pub use use_signals::{
    Purpose, SignalFiles, TDMREP_FILE, UseSignals, content_signals_refusing, rsl_refusing,
    tdmrep_file_refusing,
};
