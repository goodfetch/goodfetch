//! J-008, "Crawl through a proxy, see a Block cause reported and the Session
//! pool react", extended by spec #600 (#1137) on the `cli` surface (ADRs
//! 0021, 0063, 0075 and 0094): a person configures three of the Proving
//! ground's forward proxies (`datacenter`, `isp`, `residential`), each
//! reaching `limits.test` from an address of its own, crawls `limits/2`'s
//! list (24 items, 10 pages per address before 403), and sees each blocked
//! Session retired, its replacement opened from the next proxy kind
//! (`datacenter` -> `isp` -> `residential`), the domain keeping that kind,
//! and the oracle verifying every item of the site's list.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write the settings and fields files"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, cli as command_line};
use serde_json::{Value, json};

use crate::crawl_commands::short_idle;
use crate::fetch_tiers::settings;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The Run ID used for J-008's crawl.
const RUN_ID: &str = "j008-proxy-escalation";

/// The environment variable holding the forward proxies' password.
const PASSWORD_VARIABLE: &str = "PROVING_GROUND_PROXY_PASSWORD";

/// The fields each item's Record holds, read from its item page.
const ITEM_FIELDS: [&str; 2] = ["id", "name"];

/// The named fields the crawl reads from each page, as `--fields` takes
/// them: each item's ID from its article's attribute, and its name.
fn fields_read() -> Value {
    json!({
        "id": {
            "locator": { "kind": "css", "selector": "article.item" },
            "attribute": "data-id",
        },
        "name": { "locator": { "kind": "css", "selector": "article.item .name" } },
    })
}

/// User configuration declaring the `list` Proxy provider with the Proving
/// ground's `datacenter`, `isp`, and `residential` forward proxies.
fn list_configuration(proving_ground: &goodfetch_test_support::ProvingGround) -> String {
    let fixtures = &proving_ground.description().fixtures;
    let server = |name: &str, kind: &str| {
        format!(
            "{{ server = \"http://{}\", kind = \"{kind}\" }}",
            fixtures.proxies[name]
        )
    };
    format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nservers = [{}, {}, {}]\n",
        fixtures.proxy_credentials.username,
        server("datacenter", "datacenter"),
        server("isp", "isp"),
        server("residential", "residential"),
    )
}

/// The Records of `RUN_ID`'s item pages, each its [`ITEM_FIELDS`].
fn item_records<C>(run: &JourneyRun<C>, password: &str) -> Vec<Value> {
    let read = command_line(run.env(), run.engine())
        .env(PASSWORD_VARIABLE, password)
        .args(["run", "records", RUN_ID, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();
    read.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .filter(|read| read["record"]["value"]["id"].is_string())
        .map(|read| {
            ITEM_FIELDS
                .iter()
                .map(|field| ((*field).to_owned(), read["record"]["value"][*field].clone()))
                .collect::<serde_json::Map<String, Value>>()
                .into()
        })
        .collect()
}

/// Starts the J-008 crawl on `start` through the CLI and follows its
/// outcomes to completion.
fn crawl_list<C>(
    run: &JourneyRun<C>,
    start: &str,
    fields: &Path,
    settings_path: &Path,
    password: &str,
) -> Vec<Value> {
    let started = command_line(run.env(), run.engine())
        .env(PASSWORD_VARIABLE, password)
        .args([
            "crawl",
            "start",
            start,
            "--run-id",
            RUN_ID,
            "--format",
            "markdown",
            "--links",
            "scope",
            "--fields",
            fields.to_str().unwrap(),
            "--settings",
            settings_path.to_str().unwrap(),
            "--json",
        ])
        .json("RunId")
        .unwrap();
    assert_eq!(started.documents, vec![json!(RUN_ID)]);
    command_line(run.env(), run.engine())
        .env(PASSWORD_VARIABLE, password)
        .args(["run", "outcomes", RUN_ID, "--follow", "--json"])
        .json_lines("CrawlRequestOutcome")
        .unwrap()
        .documents
}

/// Checks that the Proving ground's relays for `2.limits.test` escalated
/// monotonically through `datacenter` -> `isp` -> `residential` and kept
/// each escalated kind.
fn assert_monotonic_escalation(proving_ground: &goodfetch_test_support::ProvingGround) {
    let relays = proving_ground.observations().unwrap()["relays"]
        .as_array()
        .unwrap()
        .clone();
    let sequence: Vec<&str> = relays
        .iter()
        .filter(|relay| {
            relay["outcome"] == json!("relayed")
                && relay["target"].as_str().is_some_and(|target| {
                    target.contains("2.limits.test") && !target.ends_with("/robots.txt")
                })
        })
        .map(|relay| relay["proxy"].as_str().unwrap())
        .collect();
    let distinct: BTreeSet<&str> = sequence.iter().copied().collect();
    assert_eq!(
        distinct,
        BTreeSet::from(["datacenter", "isp", "residential"]),
        "expected escalation across datacenter -> isp -> residential, saw {sequence:?}"
    );
    let first_isp = sequence.iter().position(|&p| p == "isp").unwrap();
    let first_res = sequence.iter().position(|&p| p == "residential").unwrap();
    assert!(
        sequence[..first_isp].iter().all(|&p| p == "datacenter"),
        "expected only datacenter before isp escalation: {sequence:?}"
    );
    assert!(
        sequence[first_isp..first_res].iter().all(|&p| p == "isp"),
        "expected domain to keep isp after escalating: {sequence:?}"
    );
    assert!(
        sequence[first_res..].iter().all(|&p| p == "residential"),
        "expected domain to keep residential after escalating: {sequence:?}"
    );
}

/// Covers: story:8-218, story:8-219, row:SA-22, row:ST-08, row:ST-09, row:CO-12, setting:escalation.proxy_kinds
#[test]
fn cli() {
    let mut run = JourneyRun::start("J-008", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-008's cli variant starts");
    let mut settings_path = PathBuf::new();
    let mut fields_path = PathBuf::new();
    let mut password = String::new();
    let mut items: Vec<Value> = Vec::new();
    let mut outcomes: Vec<Value> = Vec::new();
    short_idle(run.env());

    run.step(
        "Give the Session pool three of the Proving ground's forward proxies, each reaching the site from an address of its own",
        |run| {
            let proving_ground = run.proving_ground().expect("J-008 has a Proving ground");
            password = proving_ground
                .description()
                .fixtures
                .proxy_credentials
                .password
                .clone();
            run.env()
                .user_configuration(&list_configuration(proving_ground));
            let crawl_settings = json!({
                "fetch": { "tier": "http" },
                "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
                "session": { "pool": { "max_size": 1 } },
                "proxy": { "provider": "list" },
                "crawl": { "retry": { "max": 0 } },
            });
            settings_path = run.env().folder().join("settings.json");
            std::fs::write(
                &settings_path,
                settings(proving_ground, &crawl_settings).to_string(),
            )
            .unwrap();
            fields_path = run.env().folder().join("fields.json");
            std::fs::write(&fields_path, fields_read().to_string()).unwrap();
        },
    );
    run.step("Crawl the site's list through the proxies", |run| {
        let proving_ground = run.proving_ground().expect("J-008 has a Proving ground");
        let start = proving_ground.start_url("limits/2").unwrap();
        outcomes = crawl_list(run, &start, &fields_path, &settings_path, &password);
        items = item_records(run, &password);
    });

    run.check(
        "The run reports the Block cause of each address the site flagged",
        |run| {
            let proving_ground = run.proving_ground().expect("J-008 has a Proving ground");
            let blocked_on_server = proving_ground
                .received_requests()
                .unwrap()
                .of_the_case()
                .filter(|request| request.host == "2.limits.test" && request.status == Some(403))
                .count();
            assert!(
                blocked_on_server >= 2,
                "expected at least 2 flagged-address 403s on 2.limits.test, saw {blocked_on_server}; items={}, outcomes={outcomes:?}",
                items.len()
            );
        },
    );
    run.check(
        "The Session pool retires each Session that met a flagged address, opens its replacement from the next proxy kind, and the domain keeps that kind",
        |run| {
            let proving_ground = run.proving_ground().expect("J-008 has a Proving ground");
            assert_monotonic_escalation(proving_ground);
        },
    );
    run.expect_task(
        "The oracle verifies the Crawl's Records",
        &json!({ "status": "success", "records": items }),
    );
    let _ = command_line(run.env(), FOXHOUND)
        .args(["daemon", "stop"])
        .run();
    run.finish()
        .expect("J-008's document is written")
        .assert_proved();
}
