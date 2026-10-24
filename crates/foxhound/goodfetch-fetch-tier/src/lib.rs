//! Foxhound's fetch tiers (spec #579; ADRs 0020, 0087, 0093, 0118, 0119
//! and 0129): the tiers that fetch a crawl request's document without the
//! browser, and the page they hand its handler. The fetch driver
//! (`driver`) owns what every tier shares, the transport-failure enum and
//! what each of its variants leads to, the engine's timeouts and the
//! decoding of a body's content codings (`decoding`); each tier that sends
//! HTTP itself fills its one primitive, `send(hop)`, with a transport
//! (`transports`: reqwest in `standard` and `identified` Sessions, `wreq`
//! in HTTP-only `stealth` ones, presenting the Session's HTTP profile,
//! `http_profile`). The
//! `tiers` module is the crate's one composition value (ADR 0087):
//! [`FetchTiers`], which serves a crawl request on the tier `fetch.tier`
//! names and hands its handler a Collie page served from the document.
//! Its own crate under `crates/foxhound/`, since only Foxhound links it
//! and Collie's `cargo-deny` ban list keeps its HTTP transports out of
//! Collie's executable (STYLE.md, reason 2).

mod canonical;
mod choice;
mod decoding;
mod driver;
mod gate;
mod hedging;
mod http_profile;
mod redundant;
mod tiers;
mod transports;

pub use choice::{Forced, LabelTiers, SetTier, TierChoice};
pub use driver::{
    Body, Deadlines, Fetched, Hop, Reuse, Sending, Sent, Transport, TransportFailure, fetch,
};
pub use gate::{HandOver, RobotsTxtFetch};
pub use http_profile::HttpProfile;
pub use redundant::Requester;
pub use tiers::{FetchTiers, HandedOver, HttpTier, Signed, record_tier};
pub use transports::{Reqwest, TransportConfig, TransportUnavailable, Wreq};
