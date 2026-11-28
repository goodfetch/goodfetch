//! J-023, "Hold a person's Limit across a three-member Daemon group, with an
//! Attached engine beside a Group member", on the `cli` surface (ADRs 0041,
//! 0055, 0063 and 0075, issue 258): forms a three-member Daemon group with
//! accounts and a person account, sets a monthly person `requests` Limit
//! below the Proving ground catalog's page count, runs scripts concurrently on
//! all three Group members and an Attached engine beside one of them until the
//! Limit stops them, and verifies every new script on all four engines is
//! refused with `limit_reached` naming the person scope and the month.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps write configuration, inspect the redirect row and run concurrent scripts"
)]

use std::path::Path;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use goodfetch_engine::accounts::AccountsStore;
use goodfetch_engine::ledger::Ledger;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::ACCOUNTS_STORE;
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::{Attached, DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, JourneyRun, ProvingGround, RuntimeFile, TestEnv, browser_cache,
    cli as run_cli,
};
use serde_json::{Map, Value, json};

use crate::daemon::{StopsTheDaemon, runtime_file};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Monthly person Limit on requests: below the catalog's 12 product pages and
/// large enough for all four engines to take Slices concurrently.
const PERSON_MONTH_LIMIT: u64 = 8;

/// Catalog product page numbers each script iterates through.
const CATALOG_PAGES: std::ops::RangeInclusive<u32> = 1..=12;

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
    env.user_configuration("daemon.idle_timeout = 60000\naccounts.enabled = true\n");
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

fn run_daemon_script(
    mut client: DaemonClient,
    run_id: &str,
    settings: &Value,
    urls: &[String],
    ready: &Barrier,
    go: &Barrier,
) -> u64 {
    let started = client.call(
        1,
        "run.start",
        &json!({ "run_id": run_id, "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{run_id}: {started}");
    let page = client.call(2, "run.new_page", &json!({ "run_id": run_id }));
    let page_id = page["result"]["page_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{run_id}: {page}"))
        .to_owned();
    ready.wait();
    go.wait();

    let mut visited = 0_u64;
    let mut next_id = 3_i64;
    for url in urls {
        let went = client.call(
            next_id,
            "page.goto",
            &json!({ "page_id": page_id, "url": url }),
        );
        next_id += 1;
        if went.get("result").is_some() {
            visited += 1;
        } else {
            break;
        }
    }
    let _ = client.call(next_id, "run.finish", &json!({ "run_id": run_id }));
    visited
}

fn run_attached_script(
    mut engine: Attached,
    run_id: &str,
    settings: &Value,
    urls: &[String],
    ready: &Barrier,
    go: &Barrier,
) -> u64 {
    let hello = engine.open(&Hello::needing(Product::Collie).surface(Surface::Cli));
    assert!(hello.get("result").is_some(), "{run_id}: {hello}");
    let started = engine.ask(
        1,
        "run.start",
        &json!({ "run_id": run_id, "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{run_id}: {started}");
    let page = engine.ask(2, "run.new_page", &json!({ "run_id": run_id }));
    let page_id = page["result"]["page_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{run_id}: {page}"))
        .to_owned();
    ready.wait();

    // Take the Attached engine's first Slice through worker-a before releasing
    // the full concurrent race so scheduler jitter cannot starve it to 0 pages.
    let mut visited = 0_u64;
    let mut next_id = 3_i64;
    if let Some(first) = urls.first() {
        let went = engine.ask(
            next_id,
            "page.goto",
            &json!({ "page_id": page_id, "url": first }),
        );
        next_id += 1;
        if went.get("result").is_some() {
            visited += 1;
        }
    }
    go.wait();

    for url in urls.iter().skip(1) {
        let went = engine.ask(
            next_id,
            "page.goto",
            &json!({ "page_id": page_id, "url": url }),
        );
        next_id += 1;
        if went.get("result").is_some() {
            visited += 1;
        } else {
            break;
        }
    }
    let _ = engine.ask(next_id, "run.finish", &json!({ "run_id": run_id }));
    let _ = engine.finish();
    visited
}

fn check_refused_limit_reached(reply: &Value, label: &str) {
    let data = &reply["error"]["data"];
    assert_eq!(
        (
            data["code"].clone(),
            data["scope"]["kind"].clone(),
            data["period"]["every"].clone(),
        ),
        (json!("limit_reached"), json!("person"), json!("month")),
        "{label} was not refused with limit_reached on person/month: {reply}"
    );
}

/// Covers: story:3-115
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-023", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-023's cli variant starts");
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

    let mut attached_visited = 0_u64;
    let mut total_visited = 0_u64;
    let mut worker_a_redirect_member: Option<String> = None;
    let mut refusals: Vec<(&'static str, Value)> = Vec::new();

    run.step(
        "Form a Daemon group of three Daemons, each with its own Goodfetch home, with accounts, and give a person an account on it",
        |_run| {
            configure_member_home(&lead, true);
            configure_member_home(&worker_a, false);
            configure_member_home(&worker_b, false);

            let formed = cli_json(
                &lead,
                &cache,
                &["group", "init", "--name", "lead", "--peer-address", "127.0.0.1:0"],
                "GroupFormed",
            );
            let peer_address = formed["peer_address"].as_str().unwrap().to_owned();

            for (worker_env, member_name) in [(&worker_a, "worker-a"), (&worker_b, "worker-b")] {
                let minted = cli_json(&lead, &cache, &["group", "token"], "JoinTokenMinted");
                let _joined = cli_json(
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
                        minted["authority_key"].as_str().unwrap(),
                        "--name",
                        member_name,
                        "--peer-address",
                        "127.0.0.1:0",
                    ],
                    "GroupJoined",
                );
                wait_for_replicated_policy(worker_env, &cache);
            }

            // Verify accounts are enabled on every Group member and create the person account.
            for member_env in [&lead, &worker_a, &worker_b] {
                let mut client = open_daemon(member_env);
                let status = client.call(1, "daemon.status", &json!({}));
                assert_eq!(status["result"]["accounts"], json!(true), "{status}");
            }
            let lead_accounts = HomeLayout::of(lead.home())
                .per_major()
                .join(ACCOUNTS_STORE);
            AccountsStore::open(&lead_accounts, None)
                .unwrap()
                .make_account("Alice", "2026-01-05T09:00:00Z".parse().unwrap())
                .unwrap();
        },
    );

    run.step(
        "Set the person's monthly Limit below what extracting every product page of the catalog uses",
        |_run| {
            let set = run_cli(&lead, COLLIE)
                .env(BROWSERS_PATH_VARIABLE, &cache)
                .args([
                    "usage",
                    "set",
                    "--daemon",
                    "--scope",
                    "person",
                    "--usage-kind",
                    "requests",
                    "--period",
                    r#"{"every":"month","time_zone":"UTC"}"#,
                    "--amount",
                    &PERSON_MONTH_LIMIT.to_string(),
                ])
                .run()
                .unwrap();
            assert_eq!(set.exit, Some(0), "{set}");
        },
    );

    run.step(
        "As that person, run scripts on each Group member and on an Attached engine beside one of them at once, each extracting the catalog's product pages one after another, until they stop",
        |run| {
            let proving_ground = run
                .proving_ground()
                .expect("J-023 starts its Proving ground");
            let settings = resolving(proving_ground);
            let urls: Vec<String> = CATALOG_PAGES
                .map(|n| proving_ground.url("catalog.test", &format!("/products/P-{n:04}")))
                .collect();

            worker_a_redirect_member = Ledger::at(worker_a.home().join("usage-ledger.sqlite"), None)
                .external_group_redirect()
                .unwrap()
                .map(|row| row.member_name);

            let ready = Arc::new(Barrier::new(4));
            let go = Arc::new(Barrier::new(4));
            let c_lead = open_daemon(&lead);
            let c_a = open_daemon(&worker_a);
            let c_b = open_daemon(&worker_b);
            let c_att = Attached::start_with(
                &worker_a,
                COLLIE,
                Duration::from_secs(30),
                |cmd| {
                    cmd.env(BROWSERS_PATH_VARIABLE, &cache);
                },
            );

            let (v_lead, v_a, v_b, v_att) = std::thread::scope(|scope| {
                let h_lead = {
                    let (ready, go, settings, urls) =
                        (Arc::clone(&ready), Arc::clone(&go), settings.clone(), urls.clone());
                    scope.spawn(move || {
                        run_daemon_script(c_lead, "j023-lead", &settings, &urls, &ready, &go)
                    })
                };
                let h_a = {
                    let (ready, go, settings, urls) =
                        (Arc::clone(&ready), Arc::clone(&go), settings.clone(), urls.clone());
                    scope.spawn(move || {
                        run_daemon_script(c_a, "j023-worker-a", &settings, &urls, &ready, &go)
                    })
                };
                let h_b = {
                    let (ready, go, settings, urls) =
                        (Arc::clone(&ready), Arc::clone(&go), settings.clone(), urls.clone());
                    scope.spawn(move || {
                        run_daemon_script(c_b, "j023-worker-b", &settings, &urls, &ready, &go)
                    })
                };
                let v_att =
                    run_attached_script(c_att, "j023-attached", &settings, &urls, &ready, &go);
                (
                    h_lead.join().unwrap(),
                    h_a.join().unwrap(),
                    h_b.join().unwrap(),
                    v_att,
                )
            });

            attached_visited = v_att;
            total_visited = v_lead + v_a + v_b + v_att;
        },
    );

    run.step(
        "As that person, start one more script on each Group member and on the Attached engine",
        |_run| {
            for (label, env, run_id) in [
                ("lead", &lead, "j023-more-lead"),
                ("worker-a", &worker_a, "j023-more-worker-a"),
                ("worker-b", &worker_b, "j023-more-worker-b"),
            ] {
                let mut client = open_daemon(env);
                let reply = client.call(1, "run.start", &json!({ "run_id": run_id }));
                refusals.push((label, reply));
            }
            let mut attached =
                Attached::start_with(&worker_a, COLLIE, Duration::from_secs(30), |cmd| {
                    cmd.env(BROWSERS_PATH_VARIABLE, &cache);
                });
            let hello = attached.open(&Hello::needing(Product::Collie).surface(Surface::Cli));
            assert!(hello.get("result").is_some(), "{hello}");
            let reply = attached.ask(1, "run.start", &json!({ "run_id": "j023-more-attached" }));
            let _ = attached.finish();
            refusals.push(("attached", reply));
        },
    );

    run.check(
        "The Attached engine took its slices through the Group member beside it",
        |_run| {
            assert_eq!(worker_a_redirect_member.as_deref(), Some("worker-a"));
            assert!(
                attached_visited >= 1,
                "Attached engine should have taken at least one Slice through worker-a"
            );
        },
    );

    run.check(
        "The Usage of every run together, on the Group members and the Attached engine, never passed the person's Limit",
        |_run| {
            let listed = cli_json(
                &lead,
                &cache,
                &["usage", "list", "--daemon"],
                "UsageListResult",
            );
            let person_limit = listed["limits"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["limit"]["scope"]["kind"] == "person")
                .expect("person Limit listed on Lead Daemon");
            let charged = person_limit["charged"].as_u64().unwrap();
            let held = person_limit["held"].as_u64().unwrap();
            assert!(
                total_visited <= PERSON_MONTH_LIMIT,
                "visited {total_visited} > limit {PERSON_MONTH_LIMIT}"
            );
            assert!(
                charged + held <= PERSON_MONTH_LIMIT,
                "ledger charged ({charged}) + held ({held}) > limit {PERSON_MONTH_LIMIT}: {person_limit}"
            );
            assert_eq!(
                charged, PERSON_MONTH_LIMIT,
                "person monthly Limit should be fully consumed across the four engines: {person_limit}"
            );
        },
    );

    run.check(
        "Each new script was refused with `limit_reached`, naming the person scope and the month, on every Group member and on the Attached engine",
        |_run| {
            assert_eq!(refusals.len(), 4);
            for (label, reply) in &refusals {
                check_refused_limit_reached(reply, label);
            }
        },
    );

    run.finish()
        .expect("J-023's document is written")
        .assert_proved();
}
