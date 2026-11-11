//! Foxhound's executable, driven as its Clients drive it, for what only
//! Foxhound carries (ADR 0094); everything Collie carries is tested on
//! Collie's executable. The executable is found through
//! `CARGO_BIN_EXE_goodfetch-foxhound`.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

use std::process::Command;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, DaemonClient, Hello};
use goodfetch_test_support::{Daemon, MESSAGE_DEADLINE, TestEnv};

/// Foxhound's executable, which every test drives.
const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The Datasets a crawl these tests start declares when its `crawl.start`
/// names none (ADR 0103): those their handlers push Records into, with no
/// schema or key.
const PUSHED_INTO: [&str; 4] = ["pages", "books", "items", "links"];

/// The Run file of `run_id` in `env`'s home, opened.
pub(crate) fn run_file(env: &TestEnv, run_id: &str) -> rusqlite::Connection {
    rusqlite::Connection::open(goodfetch_test_client::HomeLayout::of(env.home()).run_file(run_id))
        .unwrap()
}

/// Which of `arrived`, a host's requests in order of arrival, its robots.txt
/// first, reached it before their turn under `delay` microseconds between
/// starts. The first after robots.txt counts its delay from that file's
/// response and each later one from the start before it, so the request `n`
/// places after robots.txt leaves at least `n` delays after the file's request
/// arrived, and arrives later still: no way to the site can bring it closer, so
/// this holds with no allowance. A gap between two requests alone is not
/// judged: it also holds the difference of their ways to the site, which no
/// figure bounds on a loaded host.
pub(crate) fn before_their_turn(arrived: &[(String, u64)], delay: u64) -> Vec<String> {
    let Some((_, read)) = arrived.first() else {
        return Vec::new();
    };
    arrived
        .iter()
        .zip(0..)
        .filter(|((_, at), places)| *at < read + places * delay)
        .map(|((target, _), _)| target.clone())
        .collect()
}

/// Foxhound's Attached engine, its logs at `trace`, once it answered the
/// Python SDK's `hello`; a `crawl.start` sent through it that declares no
/// Dataset declares [`PUSHED_INTO`].
fn attached(env: &TestEnv) -> Attached {
    attached_with(env, None, |_| {})
}

/// [`attached`], `prepare` setting the engine's command first, such as the
/// browsers' folder its pages launch from, and each wait for its output
/// bounded by `deadline` when given.
fn attached_with(
    env: &TestEnv,
    deadline: Option<Duration>,
    prepare: impl FnOnce(&mut Command),
) -> Attached {
    opened(Attached::start_with(
        env,
        FOXHOUND,
        deadline.unwrap_or(MESSAGE_DEADLINE),
        prepare,
    ))
}

/// [`attached`] as `command` starts it, such as Foxhound's executable run
/// in a mount namespace of its own.
fn attached_spawned(env: &TestEnv, deadline: Option<Duration>, command: &mut Command) -> Attached {
    opened(Attached::spawned(
        env,
        deadline.unwrap_or(MESSAGE_DEADLINE),
        command,
    ))
}

/// A Client Connection to `daemon`, once it answered the Python SDK's
/// `hello` with its token.
fn on_the_daemon(daemon: &Daemon) -> DaemonClient {
    let mut client = DaemonClient::connect(daemon.endpoint());
    let hello = client.open(
        &Hello::needing(Product::Foxhound)
            .id("hello")
            .token(&daemon.description().token),
    );
    assert_eq!(hello["result"]["product"], "foxhound", "{hello}");
    client
}

fn opened(mut engine: Attached) -> Attached {
    let opened = engine.open(&Hello::needing(Product::Foxhound).id("hello"));
    assert_eq!(opened["result"]["product"], "foxhound");
    engine.declaring_by_default(&PUSHED_INTO)
}

mod addresses;
mod admission;
mod attached;
mod audit;
mod backoff_classes;
mod body_limits;
mod bounds;
mod browser;
mod browser_fetch;
mod certificates;
mod client_conformance;
mod code_free;
mod code_free_resume;
mod command_line;
mod completeness;
mod content_types;
mod crawl;
mod crawl_command_line;
mod crawl_commands;
// The crawl benchmark's scenario, which `in_memory_crawl` checks.
#[path = "../../benches/scenarios.rs"]
#[expect(
    clippy::disallowed_methods,
    reason = "the scenario runs on the test Clock and Entropy (ADR 0059)"
)]
mod crawl_scenario;
// The run-state rows' plants, which `in_memory_crawl` proves.
#[path = "../../benches/plants.rs"]
mod crawl_plants;
mod daemon;
mod dataset_pages;
mod datasets;
mod decoding;
mod discovered_api;
mod enqueue_links;
mod escalation;
mod export_formats;
mod exports;
mod extra_loopback;
mod fetch_tiers;
mod frontier;
mod governor;
mod in_memory_crawl;
mod journeys;
mod kept_responses;
mod kept_tree;
mod label_pools;
mod learned_rules;
mod light_browser;
mod limits;
mod navigation_gate;
mod no_console;
mod operator_policy;
mod outcomes;
mod pacing;
mod path_budgets;
mod payload;
mod politeness;
mod politeness_records;
mod pool_proxies;
mod provenance;
mod records;
mod redundant_requests;
mod rejections;
mod retry_failed;
mod robots_txt;
mod sample_comparison;
mod scope;
mod seeds;
mod served_calls;
mod service_manager;
mod session_pool;
mod sitemap;
mod soak;
mod sql_tab;
mod staging;
mod tier_race;
mod trap_links;
mod url_fragments;
mod use_signals;
mod waiting;
mod workers;
mod writer_lock;

/// Whether the Run warning `code` is the one a Run carries while it holds
/// to Clean-up rules older than 14 days: every Run that holds to the
/// embedded part carries it from 14 days after the part's date on, and a
/// test of the executable runs on the real calendar, which no build lets
/// it set (TESTING.md › Time). A test that is not about the Clean-up rules
/// compares the other warnings, so its outcome never depends on the date
/// it runs on; Collie's `clean_up_rules` tests prove the rule itself.
fn on_the_calendar(code: &str) -> bool {
    code == goodfetch_protocol::run::warning::CLEAN_UP_RULES_STALE
}
