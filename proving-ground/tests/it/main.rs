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
mod body_limits;
mod browser;
mod browser_builds;
mod browserless;
mod canaries;
mod canvas;
mod clients;
mod connections;
mod consistency;
mod crawl_pages;
mod crawler;
mod dns;
mod encodings;
mod extra_loopback;
mod extraction;
mod extraction_content_modes;
mod extraction_families;
mod extraction_markdown;
mod extraction_references;
mod extraction_structured;
mod facets;
mod fake_model;
mod fetch_edge;
mod fields;
mod files;
mod fonts;
mod forward_proxy;
mod google_hosts;
mod hostile;
mod identity;
mod incidents;
mod key_events;
mod languages;
mod layout;
mod limits;
mod links;
mod live_updates;
mod login;
mod model_recordings;
mod network;
mod observations;
mod oracle;
mod pacing;
mod page_load;
mod pages;
mod parameters;
mod plain_hosts;
mod portal;
mod printing;
mod protection;
mod proving_ground;
mod raw_clients;
mod reading;
mod rendered;
mod replay;
mod rungs;
mod scale;
mod schedule;
mod scope;
mod serve;
mod sitemap_seed;
mod sitemaps;
mod sites;
mod statuses;
#[cfg(unix)]
mod stop;
mod task_list;
#[cfg(target_os = "linux")]
mod tc_netem;
mod tiers;
mod transport;
mod traps;
