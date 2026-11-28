//! J-024, "Read a Partitioned crawl as one run", on the `cli` surface (ADRs
//! 0054, 0055, 0063, 0075 and 0094, issue 248):
//! forms a Daemon group of three Foxhound Daemons (`member-2`, `member-3`,
//! `member-4`), starts one code-free Partitioned crawl of three Proving ground
//! sites (`catalog/1`, `extraction/1`, `parameters/1`) in three Partitions with
//! a crawl-level `requests` Limit, stops (pauses/checkpoints) the Partitioned
//! crawl as resumable once each Partition has committed Records, resumes it by
//! its Partitioned crawl ID until it finishes, reads its aggregated Run report
//! and unioned Dataset Records, and verifies all three sites against the
//! Proving ground oracle.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write configuration, wait for committed Records, and drive three live Foxhound Daemons"
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::DaemonFile;
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::{DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{
    JourneyRun, ProvingGround, RuntimeFile, TestEnv, cli as command_line,
};
use serde_json::{Map, Value, json};

use crate::journeys::j001::json_of;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");
const CRAWL_ID: &str = "j024-partitioned";
const REQUEST_LIMIT: u64 = 500;

/// Three Group member names whose SHA-256 rendezvous hash places
/// `catalog.test`, `extraction.test`, and `parameters.test` on three distinct
/// Partitions (`member-2`, `member-3`, `member-4`).
const LEAD_NAME: &str = "member-2";
const WORKER_A_NAME: &str = "member-3";
const WORKER_B_NAME: &str = "member-4";

struct StopsDaemon<'a>(&'a TestEnv);

impl Drop for StopsDaemon<'_> {
    fn drop(&mut self) {
        let _ = command_line(self.0, FOXHOUND)
            .args(["daemon", "stop"])
            .run();
    }
}

fn configure_member_home(env: &TestEnv, is_lead: bool) {
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    env.user_configuration("daemon.idle_timeout = 60000\n");
    if is_lead {
        std::fs::write(
            layout.operator_policy(),
            "version = \"0.1\"\n\n[settings.politeness]\nsharing = \"per_daemon\"\n",
        )
        .unwrap();
    }
}

fn cli_json(env: &TestEnv, args: &[&str], definition: &str) -> Value {
    let checked = command_line(env, FOXHOUND)
        .args(args)
        .arg("--json")
        .json(definition)
        .unwrap();
    assert_eq!(checked.output.exit, Some(0), "{}", checked.output);
    checked.documents[0].clone()
}

fn wait_for_replicated_policy(env: &TestEnv) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let doc = cli_json(env, &["config", "explain"], "SettingsExplanation");
        let has_sharing = doc["settings"].as_array().is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry["key"] == "politeness.sharing"
                    && entry["value"] == "per_daemon"
                    && entry["layer"] == "operator_policy"
            })
        });
        if has_sharing {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("worker did not receive replicated Operator policy in time");
}

fn wait_for_replicated_user_setting(env: &TestEnv, key: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let doc = cli_json(env, &["config", "explain"], "SettingsExplanation");
        let found = doc["settings"].as_array().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry["key"] == key && entry["layer"] == "user_configuration")
        });
        if found {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("worker did not receive replicated user setting `{key}` in time");
}

fn open_daemon(env: &TestEnv) -> DaemonClient {
    let runtime = HomeLayout::of(env.home()).daemon_file(DaemonFile::RuntimeFile);
    let found = RuntimeFile::read(&runtime).unwrap();
    let mut client = DaemonClient::connect(&found.endpoint);
    let hello = client.open(
        &Hello::needing(Product::Foxhound)
            .id("hello")
            .surface(Surface::Cli)
            .token(&found.token),
    );
    assert!(hello.get("result").is_some(), "{hello}");
    client
}

fn resolving_settings(proving_ground: &ProvingGround, delay_ms: u64) -> Value {
    let resolve: Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({
        "network": { "resolve": resolve },
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
        "fetch": { "tier": "http" },
        "frontier": { "learning": false },
        "politeness": { "domain": { "concurrency": 1, "delay": delay_ms } },
    })
}

fn product_fields() -> Value {
    json!({
        "id": { "locator": { "kind": "css", "selector": "article.product" }, "attribute": "data-id" },
        "name": { "locator": { "kind": "css", "selector": "article.product .name" } },
        "price": {
            "locator": { "kind": "css", "selector": "article.product .price" },
            "attribute": "data-price",
        },
    })
}

fn env_for_member<'a>(
    member: &str,
    lead: &'a TestEnv,
    worker_a: &'a TestEnv,
    worker_b: &'a TestEnv,
) -> &'a TestEnv {
    match member {
        LEAD_NAME => lead,
        WORKER_A_NAME => worker_a,
        WORKER_B_NAME => worker_b,
        other => panic!("unexpected Group member `{other}`"),
    }
}

fn read_records_strict(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let read = command_line(env, FOXHOUND)
        .args(["run", "records", run_id, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
        .unwrap();
    read.documents[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["record"].clone())
        .collect()
}

fn read_records(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let Ok(read) = command_line(env, FOXHOUND)
        .args(["run", "records", run_id, "--dataset", "pages", "--json"])
        .json_array("RecordRead")
    else {
        return Vec::new();
    };
    read.documents[0]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| entry["record"].clone())
        .collect()
}

fn wait_until_each_partition_has_records(
    partitions: &[(String, String)],
    lead: &TestEnv,
    worker_a: &TestEnv,
    worker_b: &TestEnv,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let all_have_records = partitions.iter().all(|(part_run_id, member)| {
            let env = env_for_member(member, lead, worker_a, worker_b);
            !read_records(env, part_run_id).is_empty()
        });
        if all_have_records {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("not every Partition committed Records in time before stop");
}

fn host_matches(record: &Value, expected_host: &str) -> bool {
    record["value"]["url"]
        .as_str()
        .is_some_and(|url| url.contains(expected_host))
}

/// Extracts `catalog/1` or `parameters/1` product records (`id`, `name`, `price`)
/// from the unioned `pages` Dataset, deduplicated by `(id, name, price)`.
fn product_records_for_host(records: &[Value], host: &str) -> Vec<Value> {
    let mut out: Vec<Value> = records
        .iter()
        .filter(|rec| host_matches(rec, host) && rec["value"]["id"].is_string())
        .map(|rec| {
            json!({
                "id": rec["value"]["id"],
                "name": rec["value"]["name"],
                "price": rec["value"]["price"],
            })
        })
        .collect();
    out.sort_by_key(|rec| rec["id"].to_string());
    out.dedup();
    out
}

/// Extracts `extraction/1` product records from each product page's JSON-LD
/// and Open Graph metadata (`--format metadata`).
fn extraction_records(records: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for rec in records {
        if !host_matches(rec, "extraction.test") {
            continue;
        }
        let Some(url) = rec["value"]["url"].as_str() else {
            continue;
        };
        if !url.contains("/products/") {
            continue;
        }
        let metadata = &rec["value"]["metadata"];
        let Some(ld) = metadata["json_ld"].as_array().and_then(|arr| arr.first()) else {
            continue;
        };
        let description = metadata["open_graph"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|tag| tag["property"] == "og:description")
            .and_then(|tag| tag["content"].as_str())
            .unwrap_or_default();
        out.push(json!({
            "id": ld["sku"],
            "name": ld["name"],
            "price": ld["offers"]["price"],
            "currency": ld["offers"]["priceCurrency"],
            "brand": ld["brand"]["name"],
            "gtin": ld["gtin13"],
            "rating": ld["aggregateRating"]["ratingValue"],
            "description": description,
        }));
    }
    out
}

/// Covers: story:3-116, story:3-119
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-024", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-024's cli variant starts");

    let lead = TestEnv::new();
    let worker_a = TestEnv::new();
    let worker_b = TestEnv::new();
    let _stop_lead = StopsDaemon(&lead);
    let _stop_a = StopsDaemon(&worker_a);
    let _stop_b = StopsDaemon(&worker_b);

    let mut started = Value::Null;
    let mut partition_pairs: Vec<(String, String)> = Vec::new();
    let mut paused_exit = None;
    let mut paused = Value::Null;
    let mut resumed = Value::Null;
    let mut report = Value::Null;
    let mut partition_reports: Vec<Value> = Vec::new();
    let mut all_records: Vec<Value> = Vec::new();
    let mut fast_settings_path = lead.folder().join("resume-settings.json");

    run.step(
        "Form a Daemon group of three Daemons, each with its own Goodfetch home",
        |_run| {
            configure_member_home(&lead, true);
            configure_member_home(&worker_a, false);
            configure_member_home(&worker_b, false);
            for env in [&lead, &worker_a, &worker_b] {
                let status = cli_json(env, &["daemon", "start"], "DaemonStatus");
                assert!(status["pid"].is_number(), "{status}");
            }
            let formed = cli_json(
                &lead,
                &[
                    "group",
                    "init",
                    "--name",
                    LEAD_NAME,
                    "--peer-address",
                    "127.0.0.1:0",
                ],
                "GroupFormed",
            );
            let peer_address = formed["peer_address"].as_str().unwrap().to_owned();
            let authority_key = formed["authority_key"].as_str().unwrap().to_owned();

            for (worker_env, member_name) in
                [(&worker_a, WORKER_A_NAME), (&worker_b, WORKER_B_NAME)]
            {
                let minted = cli_json(&lead, &["group", "token"], "JoinTokenMinted");
                let joined = cli_json(
                    worker_env,
                    &[
                        "group",
                        "join",
                        "--lead",
                        &peer_address,
                        "--token",
                        minted["token"].as_str().unwrap(),
                        "--authority-key",
                        &authority_key,
                        "--name",
                        member_name,
                        "--peer-address",
                        "127.0.0.1:0",
                    ],
                    "GroupJoined",
                );
                assert!(joined["certificate_expires_at"].is_string(), "{joined}");
                wait_for_replicated_policy(worker_env);
            }
            // Give each worker's initial headroom push a moment to register in
            // the Lead Daemon's member directory.
            std::thread::sleep(Duration::from_millis(300));
        },
    );

    run.step(
        "Start one Crawl of three sites (the catalog, the product index and the product listing) in three Partitions, with a Limit on the crawl",
        |run| {
            let proving_ground = run
                .proving_ground()
                .expect("J-024 starts its Proving ground");
            let slow_settings = resolving_settings(proving_ground, 300);
            let fast_settings = resolving_settings(proving_ground, 0);

            // Replicate network.resolve and egress.private_ranges across the
            // group so every Group member resolves Proving ground hosts.
            let mut lead_client = open_daemon(&lead);
            let written = lead_client.call(
                1,
                "config.user_write",
                &json!({
                    "changes": [
                        { "key": "network.resolve", "value": slow_settings["network"]["resolve"] },
                        { "key": "egress.private_ranges", "value": slow_settings["egress"]["private_ranges"] },
                    ]
                }),
            );
            assert!(written.get("result").is_some(), "config.user_write: {written}");
            wait_for_replicated_user_setting(&worker_a, "egress.private_ranges");
            wait_for_replicated_user_setting(&worker_b, "egress.private_ranges");

            let slow_settings_path = lead.folder().join("start-settings.json");
            fast_settings_path = lead.folder().join("resume-settings.json");
            let fields_path = lead.folder().join("fields.json");
            std::fs::write(&slow_settings_path, slow_settings.to_string()).unwrap();
            std::fs::write(&fast_settings_path, fast_settings.to_string()).unwrap();
            std::fs::write(&fields_path, product_fields().to_string()).unwrap();

            let catalog_url = proving_ground.start_url("catalog/1").unwrap();
            let extraction_url = proving_ground.start_url("extraction/1").unwrap();
            let parameters_url = proving_ground.start_url("parameters/1").unwrap();
            let limit_arg = format!("requests={REQUEST_LIMIT}");

            started = json_of(
                Path::new(FOXHOUND),
                &lead,
                &[
                    "crawl",
                    "start",
                    &catalog_url,
                    &extraction_url,
                    &parameters_url,
                    "--run-id",
                    CRAWL_ID,
                    "--partitions",
                    "3",
                    "--limit",
                    &limit_arg,
                    "--format",
                    "metadata",
                    "--links",
                    "scope",
                    "--fields",
                    fields_path.to_str().unwrap(),
                    "--settings",
                    slow_settings_path.to_str().unwrap(),
                    "--json",
                ],
            );
            partition_pairs = started["partitions"]
                .as_array()
                .unwrap_or_else(|| panic!("expected partitions array: {started}"))
                .iter()
                .map(|p| {
                    (
                        p["run_id"].as_str().unwrap().to_owned(),
                        p["member"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
        },
    );

    run.step(
        "Stop the Partitioned crawl as resumable once each Partition has committed some Records",
        |_run| {
            wait_until_each_partition_has_records(&partition_pairs, &lead, &worker_a, &worker_b);
            let output = command_line(&lead, FOXHOUND)
                .args(["run", "pause", CRAWL_ID, "--json"])
                .run()
                .unwrap();
            paused_exit = output.exit;
            paused = serde_json::from_str(&output.stdout)
                .unwrap_or_else(|error| panic!("{error}: {}{}", output.stdout, output.stderr));
        },
    );

    run.step(
        "Resume the Partitioned crawl by its ID and let it finish",
        |_run| {
            resumed = json_of(
                Path::new(FOXHOUND),
                &lead,
                &[
                    "run",
                    "resume",
                    CRAWL_ID,
                    "--settings",
                    fast_settings_path.to_str().unwrap(),
                    "--json",
                ],
            );
            report = json_of(
                Path::new(FOXHOUND),
                &lead,
                &["run", "report", CRAWL_ID, "--json"],
            );
            for (part_run_id, member) in &partition_pairs {
                let env = env_for_member(member, &lead, &worker_a, &worker_b);
                partition_reports.push(json_of(
                    Path::new(FOXHOUND),
                    env,
                    &["run", "report", part_run_id, "--json"],
                ));
            }
            all_records = read_records_strict(&lead, CRAWL_ID);
        },
    );

    run.step("Read the Partitioned crawl's report", |_run| {
        assert!(!report.is_null(), "Partitioned crawl report was read");
    });

    run.check(
        "The start returned a Partitioned crawl ID beside the three Partitions' Run IDs, each Partition on its own Group member",
        |_| {
            let members: BTreeSet<&str> = partition_pairs
                .iter()
                .map(|(_, member)| member.as_str())
                .collect();
            let run_ids: BTreeSet<&str> = partition_pairs
                .iter()
                .map(|(id, _)| id.as_str())
                .collect();
            assert_eq!(started["run_id"], json!(CRAWL_ID), "{started}");
            assert_eq!(partition_pairs.len(), 3, "{started}");
            assert_eq!(members.len(), 3, "each Partition on its own Group member: {started}");
            assert_eq!(run_ids.len(), 3, "distinct Partition Run IDs: {started}");
            assert!(!run_ids.contains(CRAWL_ID), "Partitioned crawl ID distinct from Partition Run IDs");
        },
    );

    run.check(
        "Stopping and resuming the Partitioned crawl checkpointed and resumed every Partition",
        |_| {
            assert_eq!(paused_exit, Some(20), "{paused}");
            assert_eq!(paused["run_id"], json!(CRAWL_ID), "{paused}");
            assert_eq!(paused["lifecycle"], json!("stopped"), "{paused}");
            assert_eq!(paused["checkpoint"]["run_id"], json!(CRAWL_ID), "{paused}");
            assert_eq!(resumed["run_id"], json!(CRAWL_ID), "{resumed}");
            assert_eq!(resumed["started_as"], json!("resumed"), "{resumed}");
            assert_eq!(
                resumed["partitions"].as_array().map(Vec::len),
                Some(3),
                "{resumed}"
            );
        },
    );

    run.check(
        "The Partitioned crawl's report gives one lifecycle, Verdict and Completeness from its Partitions, and its Usage as their sum",
        |_| {
            assert_eq!(report["run_id"], json!(CRAWL_ID), "{report}");
            assert_eq!(report["lifecycle"], json!("finished"), "{report}");
            assert_eq!(report["verdict"], json!("verified"), "{report}");
            assert_eq!(report["completeness"], json!("complete"), "{report}");
            let sum_requests: u64 = partition_reports
                .iter()
                .map(|r| r["usage"]["requests"].as_u64().unwrap_or(0))
                .sum();
            let agg_requests = report["usage"]["requests"].as_u64().unwrap_or(0);
            assert!(agg_requests > 0, "expected positive total requests: {report}");
            assert_eq!(
                agg_requests, sum_requests,
                "aggregated usage.requests must equal sum of Partitions' usage.requests"
            );
        },
    );

    run.check(
        "The Usage of the three Partitions together never passed the crawl's Limit",
        |_| {
            let agg_requests = report["usage"]["requests"].as_u64().unwrap_or(0);
            assert!(
                agg_requests > 0 && agg_requests <= REQUEST_LIMIT,
                "total requests {agg_requests} exceeded crawl Limit {REQUEST_LIMIT}: {report}"
            );
        },
    );

    let catalog = product_records_for_host(&all_records, "catalog.test");
    let extraction = extraction_records(&all_records);
    let parameters = product_records_for_host(&all_records, "parameters.test");

    run.expect_task(
        "The oracle verifies the Partitioned crawl's Records of the catalog",
        &json!({ "status": "success", "records": catalog }),
    );
    run.expect_task(
        "The oracle verifies the Partitioned crawl's Records of the product index",
        &json!({ "status": "success", "records": extraction }),
    );
    run.expect_task(
        "The oracle verifies the Partitioned crawl's Records of the product listing",
        &json!({ "status": "success", "records": parameters }),
    );

    run.finish()
        .expect("J-024's document is written")
        .assert_proved();
}
