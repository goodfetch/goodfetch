//! Multi-process group-wide Limits, *group redirect* row, seeded Group member
//! kill, and unreachable Lead Daemon `/readyz` + `/metrics` tests on Collie's
//! executable (ADRs 0041, 0050 and 0055, issue 258).
#![expect(
    clippy::disallowed_methods,
    reason = "multi-process tests write settings files, inspect the Usage ledger and kill the test Daemon process"
)]
#![expect(
    clippy::too_many_lines,
    reason = "the multi-process seeded-kill scenario verifies join, kill, fail-closed, restart and leave in sequence"
)]

use std::io::{BufRead as _, BufReader};
use std::process::Stdio;
use std::time::{Duration, Instant};

use goodfetch_engine::ledger::Ledger;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::{Attached, DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, RuntimeFile, TestEnv, browser_cache, cli,
};
use serde_json::{Map, Value, json};

use crate::daemon::{StopsTheDaemon, short_idle};
use crate::ops::{Browsers, get as ops_get, with_ops};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

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

fn answered(env: &TestEnv, browsers: &str, args: &[&str], definition: &str) -> Value {
    let checked = cli(env, COLLIE)
        .env(BROWSERS_PATH_VARIABLE, browsers)
        .args(args)
        .arg("--json")
        .json(definition)
        .unwrap();
    assert_eq!(checked.output.exit, Some(0), "{}", checked.output);
    checked.documents[0].clone()
}

fn allow_per_daemon_politeness(env: &TestEnv) {
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    std::fs::write(
        layout.operator_policy(),
        "version = \"0.1\"\n\n[settings.politeness]\nsharing = \"per_daemon\"\n",
    )
    .unwrap();
}

/// Kills the Group member at a seeded moment while an Attached engine beside
/// it runs against a group Limit, showing:
/// - the Attached engine never runs past the group Limit;
/// - once the Group member dies, new Slices and new runs on the Attached engine
///   fail closed with `lead_unreachable`;
/// - restarting the Group member renews the redirect row and lets runs resume;
/// - `group leave` removes the *group redirect* row from `usage-ledger.sqlite`.
///
/// Covers: story:3-112, story:3-115
#[test]
fn killed_group_member_at_seeded_moment_prevents_attached_engine_from_passing_group_limit() {
    let lead = TestEnv::new();
    let worker = TestEnv::new();
    short_idle(&lead, 60_000);
    short_idle(&worker, 60_000);
    allow_per_daemon_politeness(&lead);
    let _stop_lead = StopsTheDaemon::of(&lead, COLLIE);
    let _stop_worker = StopsTheDaemon::of(&worker, COLLIE);

    let cache = browser_cache(&lead)
        .expect("Playwright's Chrome for Testing is installed")
        .to_string_lossy()
        .into_owned();
    let proving_ground = ProvingGround::start(&lead, 258).unwrap();
    let settings = resolving(&proving_ground);

    let formed = answered(
        &lead,
        &cache,
        &[
            "group",
            "init",
            "--name",
            "lead",
            "--peer-address",
            "127.0.0.1:0",
        ],
        "GroupFormed",
    );
    let minted = answered(&lead, &cache, &["group", "token"], "JoinTokenMinted");

    // Start worker's Daemon as a tracked child process so we can kill it cleanly by PID.
    let mut command = worker.process_command(COLLIE);
    command
        .env(BROWSERS_PATH_VARIABLE, &cache)
        .args(["daemon", "launched"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let worker_proc = worker.spawn(&mut command).unwrap();
    let mut ready_line = String::new();
    BufReader::new(worker_proc.child().stdout.take().unwrap())
        .read_line(&mut ready_line)
        .unwrap();
    let _worker_runtime: RuntimeFile = serde_json::from_str(&ready_line).unwrap();

    let _joined = answered(
        &worker,
        &cache,
        &[
            "group",
            "join",
            "--lead",
            formed["peer_address"].as_str().unwrap(),
            "--token",
            minted["token"].as_str().unwrap(),
            "--authority-key",
            minted["authority_key"].as_str().unwrap(),
            "--name",
            "worker-a",
            "--peer-address",
            "127.0.0.1:0",
        ],
        "GroupJoined",
    );

    // Set group-wide install Limit = 6 on Lead Daemon (slice size = 1).
    let set = cli(&lead, COLLIE)
        .env(BROWSERS_PATH_VARIABLE, &cache)
        .args([
            "usage",
            "set",
            "--daemon",
            "--scope",
            "install",
            "--usage-kind",
            "requests",
            "--amount",
            "6",
        ])
        .run()
        .unwrap();
    assert_eq!(set.exit, Some(0), "{set}");

    // Start an Attached engine beside worker-a and visit catalog pages until the seeded kill moment.
    let kill_after = 1 + (worker.seed() % 3);
    let mut attached = Attached::start_with(&worker, COLLIE, Duration::from_secs(30), |cmd| {
        cmd.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let hello = attached.open(&Hello::needing(Product::Collie).surface(Surface::Cli));
    assert!(hello.get("result").is_some(), "{hello}");
    let started = attached.ask(
        1,
        "run.start",
        &json!({ "run_id": "seeded-kill-run", "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let page = attached.ask(2, "run.new_page", &json!({ "run_id": "seeded-kill-run" }));
    let page_id = page["result"]["page_id"].as_str().unwrap().to_owned();

    let mut next_id = 3_i64;
    let mut visited = 0_u64;
    for n in 1..=kill_after {
        let url = proving_ground.url("catalog.test", &format!("/products/P-{n:04}"));
        let went = attached.ask(
            next_id,
            "page.goto",
            &json!({ "page_id": page_id, "url": url }),
        );
        next_id += 1;
        assert!(went.get("result").is_some(), "{went}");
        visited += 1;
    }

    // Kill the Group member abruptly (`SIGKILL` / `TerminateProcess`) at the seeded moment!
    let _ = worker_proc.child().kill();
    let _ = worker_proc.child().wait();

    // Next page.goto needing a Slice MUST fail closed (never running past the group Limit).
    let url = proving_ground.url("catalog.test", "/products/P-0009");
    let blocked = attached.ask(
        next_id,
        "page.goto",
        &json!({ "page_id": page_id, "url": url }),
    );
    assert!(
        blocked.get("error").is_some(),
        "Attached engine must not take a Slice after its Group member died: {blocked}"
    );
    let _ = attached.finish();
    assert_eq!(visited, kill_after);
    assert!(visited < 6);

    // A brand-new Attached engine beside dead worker-a fails closed with `lead_unreachable`.
    let mut second = Attached::start_with(&worker, COLLIE, Duration::from_secs(30), |cmd| {
        cmd.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let hello = second.open(&Hello::needing(Product::Collie).surface(Surface::Cli));
    assert!(hello.get("result").is_some(), "{hello}");
    let refused = second.ask(1, "run.start", &json!({ "run_id": "after-kill-run" }));
    assert_eq!(
        refused["error"]["data"]["code"],
        json!("lead_unreachable"),
        "{refused}"
    );
    let _ = second.finish();

    // Verify Lead Daemon's Usage ledger never exceeded the Limit.
    let listed = answered(
        &lead,
        &cache,
        &["usage", "list", "--daemon"],
        "UsageListResult",
    );
    let entry = &listed["limits"][0];
    let charged = entry["charged"].as_u64().unwrap();
    let held = entry["held"].as_u64().unwrap();
    assert!(charged + held <= 6, "{entry}");

    // Restarting worker-a renews the redirect row so an Attached engine can start runs again.
    let restarted = cli(&worker, COLLIE)
        .env(BROWSERS_PATH_VARIABLE, &cache)
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();
    assert_eq!(restarted.exit, Some(0), "{restarted}");
    let mut third = Attached::start_with(&worker, COLLIE, Duration::from_secs(30), |cmd| {
        cmd.env(BROWSERS_PATH_VARIABLE, &cache);
    });
    let hello = third.open(&Hello::needing(Product::Collie).surface(Surface::Cli));
    assert!(hello.get("result").is_some(), "{hello}");
    let resumed_ok = third.ask(1, "run.start", &json!({ "run_id": "after-return-run" }));
    assert!(resumed_ok.get("result").is_some(), "{resumed_ok}");
    let _ = third.ask(2, "run.finish", &json!({ "run_id": "after-return-run" }));
    let _ = third.finish();

    // `group leave` removes the *group redirect* row from worker-a's per-user ledger.
    let _left = answered(&worker, &cache, &["group", "leave"], "GroupLeft");
    assert_eq!(
        Ledger::at(worker.home().join("usage-ledger.sqlite"), None)
            .external_group_redirect()
            .unwrap(),
        None,
        "group leave must remove the group redirect row"
    );
}

/// With the Lead Daemon unreachable, a Group member's `/readyz` stays HTTP 200
/// and `/metrics` reports `goodfetch_group_lead_unreachable` as 1 (ADRs 0050
/// and 0055).
///
/// Covers: story:3-112
#[test]
fn unreachable_lead_keeps_readyz_ready_and_reports_metric() {
    let lead = TestEnv::new();
    let worker = TestEnv::new();
    short_idle(&lead, 60_000);
    allow_per_daemon_politeness(&lead);
    let _stop_lead = StopsTheDaemon::of(&lead, COLLIE);

    let cache = browser_cache(&lead)
        .expect("Playwright's Chrome for Testing is installed")
        .to_string_lossy()
        .into_owned();

    let formed = answered(
        &lead,
        &cache,
        &[
            "group",
            "init",
            "--name",
            "lead",
            "--peer-address",
            "127.0.0.1:0",
        ],
        "GroupFormed",
    );
    let minted = answered(&lead, &cache, &["group", "token"], "JoinTokenMinted");

    // Start worker as a foreground Daemon with an ops listener (`/readyz`, `/metrics`).
    let (mut worker_daemon, ops_addr) = with_ops(&worker, Browsers::Installed);
    let worker_endpoint = worker_daemon.serving();
    let ops_tok =
        std::fs::read_to_string(HomeLayout::of(worker.home()).daemon_file(DaemonFile::OpsToken))
            .unwrap();
    let daemon_tok =
        std::fs::read_to_string(HomeLayout::of(worker.home()).daemon_file(DaemonFile::Token))
            .unwrap();

    let mut client = DaemonClient::connect(&worker_endpoint);
    let hello = client.open(
        &Hello::needing(Product::Collie)
            .id("hello")
            .surface(Surface::Cli)
            .token(&daemon_tok),
    );
    assert!(hello.get("result").is_some(), "{hello}");

    let joined = client.call(
        1,
        "group.join",
        &json!({
            "lead": formed["peer_address"],
            "token": minted["token"],
            "authority_key": minted["authority_key"],
            "name": "worker-ops",
            "peer_address": "127.0.0.1:0",
        }),
    );
    assert!(joined.get("result").is_some(), "{joined}");

    // Stop the Lead Daemon so it becomes unreachable.
    let stopped = cli(&lead, COLLIE).args(["daemon", "stop"]).run().unwrap();
    assert_eq!(stopped.exit, Some(0), "{stopped}");

    // Trigger a group ledger call on worker -> fails closed with `lead_unreachable`.
    let failed = client.call(2, "usage.list", &json!({}));
    assert_eq!(
        failed["error"]["data"]["code"],
        json!("lead_unreachable"),
        "{failed}"
    );

    // `/readyz` MUST stay HTTP 200 while the Lead Daemon is unreachable.
    let (ready_status, _) = ops_get(ops_addr, "/readyz", None);
    assert_eq!(ready_status, 200);

    // `/metrics` MUST report `goodfetch_group_lead_unreachable` as 1.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_unreachable = false;
    while Instant::now() < deadline {
        let (status, body) = ops_get(ops_addr, "/metrics", Some(&ops_tok));
        assert_eq!(status, 200);
        if body.lines().any(|line| {
            line.starts_with("goodfetch_group_lead_unreachable") && line.ends_with(" 1")
        }) {
            saw_unreachable = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        saw_unreachable,
        "expected goodfetch_group_lead_unreachable 1 in /metrics"
    );

    drop(client);
    assert!(worker_daemon.terminate());
}
