//! J-050, "Crawl a site with honeypot links and a labyrinth; no trap is fetched
//! and the decoy is reported", on the `cli` surface (`J-050@foxhound-cli`;
//! ADRs 0023, 0035, 0063, 0075 and 0094; issue 1157): a person crawls
//! `traps.test/hidden-links` in the browser with robots.txt off so only
//! trap-link avoidance protects the honeypots, and crawls
//! `access.test/corpus/labyrinth` with a Page check expecting `article.item`.
//! No honeypot link is fetched by the Proving ground, every visible item link
//! is fetched, and the labyrinth page ends `blocked` with Block cause `decoy`
//! in its outcomes and Run report.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the settings files for the CLI crawl"
)]

use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, ReceivedRequests, cli as command_line};
use serde_json::{Value, json};

use crate::crawl_commands::short_idle;
use crate::fetch_tiers::settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

const HONEYPOT_RUN_ID: &str = "j050-honeypots";
const LABYRINTH_RUN_ID: &str = "j050-labyrinth";
const TRAPS_HOST: &str = "traps.test";

/// Starts a CLI crawl on `start_url` with `run_id`, optional `--links scope`,
/// and `settings_path`, then follows its outcomes to completion.
fn run_cli_crawl<C>(
    run: &JourneyRun<C>,
    run_id: &str,
    start_url: &str,
    follow_links: bool,
    settings_path: &Path,
) -> Vec<Value> {
    let mut args = vec![
        "crawl", "start", start_url, "--run-id", run_id, "--format", "markdown",
    ];
    if follow_links {
        args.extend(["--links", "scope"]);
    }
    args.extend(["--settings", settings_path.to_str().unwrap(), "--json"]);
    let started = command_line(run.env(), run.engine())
        .args(&args)
        .json("RunId")
        .unwrap();
    assert_eq!(started.documents, vec![json!(run_id)]);
    command_line(run.env(), run.engine())
        .args(["run", "outcomes", run_id, "--follow", "--json"])
        .json_lines("CrawlRequestOutcome")
        .unwrap()
        .documents
}

/// Checks that no honeypot under `/hp/` was fetched on [`TRAPS_HOST`] and
/// every visible item `/items/I-0001..=I-0013` was fetched.
fn assert_honeypots_avoided(proving_ground: &goodfetch_test_support::ProvingGround) {
    let observations = proving_ground.observations().unwrap();
    let hp_count = ReceivedRequests::of(&observations)
        .of_the_case()
        .filter(|request| request.host == TRAPS_HOST && request.target.as_str().starts_with("/hp/"))
        .count();
    assert_eq!(
        hp_count, 0,
        "no honeypot link under /hp/ may be fetched by the Proving ground"
    );
    let mut fetched_items: Vec<String> = ReceivedRequests::of(&observations)
        .of_the_case()
        .filter(|request| {
            request.host == TRAPS_HOST && request.target.as_str().starts_with("/items/")
        })
        .map(|request| request.target.as_str().to_owned())
        .collect();
    fetched_items.sort();
    fetched_items.dedup();
    let expected_items: Vec<String> = (1..=13)
        .map(|number| format!("/items/I-{number:04}"))
        .collect();
    assert_eq!(
        fetched_items, expected_items,
        "every visible item link must be fetched"
    );
}

/// Covers: story:8-230, story:8-231, block-cause:decoy
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-050", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-050's cli variant starts");
    short_idle(run.env());
    let mut labyrinth_outcomes: Vec<Value> = Vec::new();
    let mut labyrinth_report = Value::Null;

    run.step(
        "Crawl the site's hidden-links page in the browser with robots.txt off so only trap-link avoidance protects the honeypots",
        |run| {
            let proving_ground = run.proving_ground().expect("J-050 has a Proving ground");
            let browser_settings = run.env().folder().join("honeypot-settings.json");
            std::fs::write(
                &browser_settings,
                settings(
                    proving_ground,
                    &json!({
                        "fetch": { "tier": "browser" },
                        "crawl": { "max_depth": 1 },
                        "site_preferences": { "respect_robots_txt": false },
                    }),
                )
                .to_string(),
            )
            .unwrap();
            let start = proving_ground.url(TRAPS_HOST, "/hidden-links");
            let _ = run_cli_crawl(run, HONEYPOT_RUN_ID, &start, true, &browser_settings);
        },
    );

    run.step(
        "Crawl the labyrinth page with a Page check expecting real item content",
        |run| {
            let proving_ground = run.proving_ground().expect("J-050 has a Proving ground");
            let labyrinth_settings = run.env().folder().join("labyrinth-settings.json");
            std::fs::write(
                &labyrinth_settings,
                settings(
                    proving_ground,
                    &json!({
                        "fetch": { "tier": "browser" },
                        "session": {
                            "page_check": {
                                "expected_page": { "kind": "css", "selector": "article.item" }
                            }
                        },
                    }),
                )
                .to_string(),
            )
            .unwrap();
            let start = proving_ground.url("access.test", "/corpus/labyrinth");
            labyrinth_outcomes =
                run_cli_crawl(run, LABYRINTH_RUN_ID, &start, false, &labyrinth_settings);
            let report_out = command_line(run.env(), run.engine())
                .args(["run", "report", LABYRINTH_RUN_ID, "--json"])
                .run()
                .unwrap();
            assert_eq!(report_out.exit, Some(0), "{report_out}");
            labyrinth_report =
                serde_json::from_str(&report_out.stdout).expect("CLI printed JSON RunReport");
        },
    );

    run.check(
        "No honeypot link is fetched by the Proving ground, and every visible item link is fetched",
        |run| {
            let proving_ground = run.proving_ground().expect("J-050 has a Proving ground");
            assert_honeypots_avoided(proving_ground);
        },
    );

    run.check(
        "The labyrinth page is reported as blocked with Block cause decoy in its outcomes and Run report",
        |_run| {
            assert_eq!(labyrinth_outcomes.len(), 1, "{labyrinth_outcomes:?}");
            assert_eq!(labyrinth_outcomes[0]["kind"], json!("blocked"));
            assert_eq!(labyrinth_outcomes[0]["cause"], json!("decoy"));
            let by_cause = labyrinth_report["requests"]["by_block_cause"]
                .as_array()
                .expect("Run report requests.by_block_cause");
            assert!(
                by_cause
                    .iter()
                    .any(|entry| entry["name"] == json!("decoy") && entry["count"] == json!(1)),
                "expected decoy count 1 in Run report requests.by_block_cause: {labyrinth_report}"
            );
        },
    );

    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-050's document is written")
        .assert_proved();
}
