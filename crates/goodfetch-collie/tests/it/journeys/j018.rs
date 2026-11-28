//! J-018, "Form a Daemon group and remove a Group member while its runs go on",
//! on the `cli` surface (ADRs 0055, 0063 and 0075, issues 242 and 245):
//! starts three Collie Daemons with their own Goodfetch homes, forms one Daemon
//! group over mutual TLS under the group's own certificate authority, starts
//! runs across the group extracting the Proving ground catalog's product pages
//! into each Run's Key-value store, removes a Group member holding live runs
//! via `group leave` so it drains as a removal and hands each run's Checkpoint
//! over the peer link to a remaining Group member, and finishes every run on
//! the remaining Group members with every product page extracted, none lost
//! and none twice.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write configuration and drive three live Collie Daemons"
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::{DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, JourneyRun, ProvingGround, RuntimeFile, TestEnv, browser_cache,
    cli as run_cli,
};
use serde_json::{Map, Value, json};

use crate::daemon::{StopsTheDaemon, runtime_file};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Product pages each Run extracts (split across pre-removal and post-handoff).
const TOTAL_PAGES: u32 = 4;
const PRE_REMOVAL_PAGES: u32 = 2;

fn resolving(proving_ground: &ProvingGround) -> Value {
    let resolve: Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({
        "network": { "resolve": resolve },
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
    })
}

fn cli_json(env: &TestEnv, browsers: &str, args: &[&str], definition: &str) -> Value {
    let checked = run_cli(env, COLLIE)
        .env(BROWSERS_PATH_VARIABLE, browsers)
        .args(args)
        .arg("--json")
        .json(definition)
        .unwrap();
    assert_eq!(checked.output.exit, Some(0), "{}", checked.output);
    checked.documents[0].clone()
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

fn wait_for_replicated_policy(env: &TestEnv, browsers: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let doc = cli_json(env, browsers, &["config", "explain"], "SettingsExplanation");
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

fn wait_for_replicated_user_setting(env: &TestEnv, browsers: &str, key: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let doc = cli_json(env, browsers, &["config", "explain"], "SettingsExplanation");
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
    let found = RuntimeFile::read(&runtime_file(env)).unwrap();
    let mut client = DaemonClient::connect(&found.endpoint);
    let hello = client.open(
        &Hello::needing(Product::Collie)
            .id("hello")
            .surface(Surface::Cli)
            .token(&found.token),
    );
    assert!(hello.get("result").is_some(), "{hello}");
    client
}

fn run_list_on_lead(lead: &TestEnv) -> Vec<Value> {
    let mut client = open_daemon(lead);
    let reply = client.call(1, "run.list", &json!({}));
    assert!(reply.get("result").is_some(), "run.list: {reply}");
    reply["result"]["runs"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn wait_for_run_owners_on_lead(lead: &TestEnv, expected: &[(&str, &str)]) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let rows = run_list_on_lead(lead);
        let all_match = expected.iter().all(|(run_id, owner)| {
            rows.iter()
                .any(|row| row["run_id"] == *run_id && row["owner"]["group_member"] == *owner)
        });
        if all_match {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let rows = run_list_on_lead(lead);
    panic!("Lead group Run index did not match {expected:?} in time; got {rows:?}");
}

fn extract_pages_into_run(
    client: &mut DaemonClient,
    run_id: &str,
    urls: &[(u32, String)],
    next_id: &mut i64,
) {
    let page = client.call(*next_id, "run.new_page", &json!({ "run_id": run_id }));
    *next_id += 1;
    let page_id = page["result"]["page_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{run_id} new_page: {page}"))
        .to_owned();
    for (num, url) in urls {
        let went = client.call(
            *next_id,
            "page.goto",
            &json!({ "page_id": page_id, "url": url }),
        );
        *next_id += 1;
        assert!(
            went.get("result").is_some(),
            "{run_id} goto page {num}: {went}"
        );
        let saved = client.call(
            *next_id,
            "key_value_store.set",
            &json!({
                "run_id": run_id,
                "key": format!("product-{num:04}"),
                "value": { "page": num, "url": url },
            }),
        );
        *next_id += 1;
        assert!(
            saved.get("result").is_some(),
            "{run_id} kv.set page {num}: {saved}"
        );
    }
    let closed = client.call(*next_id, "page.close", &json!({ "page_id": page_id }));
    *next_id += 1;
    assert!(
        closed.get("result").is_some(),
        "{run_id} page.close: {closed}"
    );
}

fn read_run_kv_keys(client: &mut DaemonClient, run_id: &str, next_id: &mut i64) -> Vec<String> {
    let call_id = *next_id;
    *next_id += 1;
    client.send(&json!({
        "jsonrpc": "2.0",
        "id": call_id,
        "method": "key_value_store.keys",
        "params": { "run_id": run_id },
    }));
    client.send(&json!({
        "jsonrpc": "2.0",
        "method": "$/credit",
        "params": { "id": call_id, "credits": 100 },
    }));
    let mut keys = Vec::new();
    while let Some(msg) = client.next() {
        if msg["method"] == "$/key" && msg["params"]["id"] == json!(call_id) {
            if let Some(key) = msg["params"]["key"].as_str() {
                keys.push(key.to_owned());
            }
            continue;
        }
        if msg["id"] == json!(call_id) {
            assert!(
                msg.get("result").is_some(),
                "{run_id} key_value_store.keys: {msg}"
            );
            return keys;
        }
    }
    panic!("{run_id} key_value_store.keys ended before reply");
}

fn env_for_member<'a>(member: &str, lead: &'a TestEnv, worker_b: &'a TestEnv) -> &'a TestEnv {
    match member {
        "lead" => lead,
        "worker-b" => worker_b,
        other => panic!("unexpected remaining Group member `{other}`"),
    }
}

/// Covers: story:3-106, story:3-111
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-018", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-018's cli variant starts");
    let cache = browser_cache(run.env())
        .expect("Playwright's Chrome for Testing is installed")
        .to_string_lossy()
        .into_owned();

    let lead = TestEnv::new();
    let worker_a = TestEnv::new();
    let worker_b = TestEnv::new();
    let _stop_lead = StopsTheDaemon::of(&lead, COLLIE);
    let _stop_a = StopsTheDaemon::of(&worker_a, COLLIE);
    let _stop_b = StopsTheDaemon::of(&worker_b, COLLIE);

    let mut authority_key = String::new();
    let mut joined_ok = 0_usize;
    let mut initial_index_ids: BTreeSet<String> = BTreeSet::new();
    let mut left_doc = Value::Null;
    let mut all_urls: Vec<(u32, String)> = Vec::new();
    let mut final_owners: Vec<(String, String)> = Vec::new();
    let mut final_pages: Vec<(String, Vec<String>)> = Vec::new();
    let mut run_settings = Value::Null;

    run.step(
        "Start three Daemons, each with its own Goodfetch home",
        |_run| {
            configure_member_home(&lead, true);
            configure_member_home(&worker_a, false);
            configure_member_home(&worker_b, false);
            for env in [&lead, &worker_a, &worker_b] {
                let status = cli_json(env, &cache, &["daemon", "start"], "DaemonStatus");
                assert!(status["pid"].is_number(), "{status}");
            }
        },
    );

    run.step(
        "Form a Daemon group: initialize it on the first Daemon, make a join token and join the other two",
        |_run| {
            let formed = cli_json(
                &lead,
                &cache,
                &["group", "init", "--name", "lead", "--peer-address", "127.0.0.1:0"],
                "GroupFormed",
            );
            let peer_address = formed["peer_address"].as_str().unwrap().to_owned();
            authority_key = formed["authority_key"].as_str().unwrap().to_owned();

            for (worker_env, member_name) in [(&worker_a, "worker-a"), (&worker_b, "worker-b")] {
                let minted = cli_json(&lead, &cache, &["group", "token"], "JoinTokenMinted");
                assert_eq!(minted["authority_key"].as_str().unwrap(), authority_key);
                let joined = cli_json(
                    worker_env,
                    &cache,
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
                wait_for_replicated_policy(worker_env, &cache);
                joined_ok += 1;
            }
        },
    );

    run.step(
        "Start runs across the Daemon group, each extracting the catalog's product pages one after another",
        |run| {
            let proving_ground = run
                .proving_ground()
                .expect("J-018 starts its Proving ground");
            run_settings = resolving(proving_ground);
            all_urls = (1..=TOTAL_PAGES)
                .map(|n| {
                    (
                        n,
                        proving_ground.url("catalog.test", &format!("/products/P-{n:04}")),
                    )
                })
                .collect();

            // Write network.resolve + egress.private_ranges into Lead's user
            // configuration so every Group member inherits it on Checkpoint resume.
            let mut lead_client = open_daemon(&lead);
            let written = lead_client.call(
                1,
                "config.user_write",
                &json!({
                    "changes": [
                        { "key": "network.resolve", "value": run_settings["network"]["resolve"] },
                        { "key": "egress.private_ranges", "value": run_settings["egress"]["private_ranges"] },
                    ]
                }),
            );
            assert!(written.get("result").is_some(), "config.user_write: {written}");
            wait_for_replicated_user_setting(&worker_b, &cache, "egress.private_ranges");

            let first_half: Vec<(u32, String)> = all_urls
                .iter()
                .take(PRE_REMOVAL_PAGES as usize)
                .cloned()
                .collect();

            // Start two runs pinned to worker-a and one pinned to worker-b,
            // extracting the first half of the product pages into each Run's KV store.
            let mut client_a = open_daemon(&worker_a);
            let mut id_a = 1_i64;
            for run_id in ["j018-a1", "j018-a2"] {
                let started = client_a.call(
                    id_a,
                    "run.start",
                    &json!({
                        "run_id": run_id,
                        "group_member": "worker-a",
                        "settings": run_settings,
                    }),
                );
                id_a += 1;
                assert!(started.get("result").is_some(), "{run_id}: {started}");
                extract_pages_into_run(&mut client_a, run_id, &first_half, &mut id_a);
            }

            let mut client_b = open_daemon(&worker_b);
            let mut id_b = 1_i64;
            let started_b = client_b.call(
                id_b,
                "run.start",
                &json!({
                    "run_id": "j018-b1",
                    "group_member": "worker-b",
                    "settings": run_settings,
                }),
            );
            id_b += 1;
            assert!(started_b.get("result").is_some(), "j018-b1: {started_b}");
            extract_pages_into_run(&mut client_b, "j018-b1", &first_half, &mut id_b);

            wait_for_run_owners_on_lead(
                &lead,
                &[
                    ("j018-a1", "worker-a"),
                    ("j018-a2", "worker-a"),
                    ("j018-b1", "worker-b"),
                ],
            );
            for row in run_list_on_lead(&lead) {
                if let Some(id) = row["run_id"].as_str() {
                    initial_index_ids.insert(id.to_owned());
                }
            }
        },
    );

    run.step(
        "Remove a Group member that holds runs while they go on",
        |_run| {
            left_doc = cli_json(&worker_a, &cache, &["group", "leave"], "GroupLeft");
        },
    );

    run.step("Wait for every run to finish", |_run| {
        let rows = run_list_on_lead(&lead);
        for run_id in ["j018-a1", "j018-a2", "j018-b1"] {
            let row = rows
                .iter()
                .find(|r| r["run_id"] == run_id)
                .unwrap_or_else(|| panic!("{run_id} missing from Lead Run index: {rows:?}"));
            let owner = row["owner"]["group_member"]
                .as_str()
                .unwrap_or_else(|| panic!("{run_id} has no group_member owner: {row}"))
                .to_owned();
            assert_ne!(
                owner, "worker-a",
                "{run_id} must be reassigned off removed worker-a: {row}"
            );
            final_owners.push((run_id.to_owned(), owner.clone()));

            let owner_env = env_for_member(&owner, &lead, &worker_b);
            let mut client = open_daemon(owner_env);
            let mut next_id = 1_i64;
            let already_saved: BTreeSet<String> =
                read_run_kv_keys(&mut client, run_id, &mut next_id)
                    .into_iter()
                    .collect();
            let remaining: Vec<(u32, String)> = all_urls
                .iter()
                .filter(|(n, _)| !already_saved.contains(&format!("product-{n:04}")))
                .cloned()
                .collect();
            assert_eq!(
                already_saved.len(),
                PRE_REMOVAL_PAGES as usize,
                "{run_id} should carry its {PRE_REMOVAL_PAGES} pre-removal pages across the Checkpoint handoff; got {already_saved:?}"
            );
            extract_pages_into_run(&mut client, run_id, &remaining, &mut next_id);

            let keys = read_run_kv_keys(&mut client, run_id, &mut next_id);
            let finished = client.call(next_id, "run.finish", &json!({ "run_id": run_id }));
            assert!(finished.get("result").is_some(), "{run_id} finish: {finished}");
            final_pages.push((run_id.to_owned(), keys));
        }
    });

    run.check(
        "The three Daemons are Group members of one Daemon group, with the first as its Lead Daemon, linked by mutual TLS under the group's own certificate authority",
        |_run| {
            assert!(authority_key.starts_with("sha256/"));
            assert_eq!(joined_ok, 2);
        },
    );

    run.check(
        "Every run started on any Group member is listed in the Daemon group's Run index",
        |_run| {
            for id in ["j018-a1", "j018-a2", "j018-b1"] {
                assert!(
                    initial_index_ids.contains(id),
                    "expected {id} in Lead Run index, got {initial_index_ids:?}"
                );
            }
        },
    );

    run.check(
        "The removed Group member drained as a removal, not a restart, and handed each of its runs' Checkpoints to another Group member",
        |_run| {
            assert_eq!(left_doc["name"], json!("worker-a"), "{left_doc}");
            let handed: BTreeSet<String> = left_doc["handed_off"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            assert_eq!(
                handed,
                BTreeSet::from(["j018-a1".to_owned(), "j018-a2".to_owned()]),
                "worker-a must hand off both of its runs on group leave: {left_doc}"
            );
        },
    );

    run.check(
        "Every run finished on a remaining Group member with every product page extracted, none lost and none twice",
        |_run| {
            let expected_keys: BTreeSet<String> = (1..=TOTAL_PAGES)
                .map(|n| format!("product-{n:04}"))
                .collect();
            assert_eq!(final_owners.len(), 3);
            for (run_id, owner) in &final_owners {
                assert!(
                    owner == "lead" || owner == "worker-b",
                    "{run_id} finished on unexpected owner {owner}"
                );
            }
            assert_eq!(final_pages.len(), 3);
            for (run_id, keys) in &final_pages {
                assert_eq!(
                    keys.len(),
                    TOTAL_PAGES as usize,
                    "{run_id} expected {TOTAL_PAGES} unique extracted pages, got {keys:?}"
                );
                let unique: BTreeSet<String> = keys.iter().cloned().collect();
                assert_eq!(unique, expected_keys, "{run_id} pages mismatch");
            }
        },
    );

    run.finish()
        .expect("J-018's document is written")
        .assert_proved();
}
