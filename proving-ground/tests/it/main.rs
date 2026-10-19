//! The Proving ground as a black box (spec #152, Seam 1): each test starts
//! the binary on a seed and drives it with a plain HTTP client, never an
//! engine crate nor either Product, so the oracle is never tested with a tool
//! it judges.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests start the Proving ground's binary as a child process"
)]

mod access;
mod actionability;
mod adversarial;
mod agent;
mod browser;
mod browserless;
mod canvas;
mod clients;
mod connections;
mod crawl_pages;
mod crawler;
mod dns;
mod encodings;
mod extra_loopback;
mod extraction;
mod extraction_families;
mod fake_model;
mod fetch_edge;
mod files;
mod forward_proxy;
mod identity;
mod incidents;
mod key_events;
mod languages;
mod layout;
mod limits;
mod live_updates;
mod login;
mod model_recordings;
mod network;
mod observations;
mod oracle;
mod pages;
mod parameters;
mod portal;
mod printing;
mod protection;
mod proving_ground;
mod raw_clients;
mod reading;
mod rendered;
mod replay;
mod scale;
mod schedule;
mod serve;
mod statuses;
mod task_list;
#[cfg(target_os = "linux")]
mod tc_netem;
mod transport;
mod traps;
