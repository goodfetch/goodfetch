//! Partition handoff integration tests on real Foxhound Daemons against the
//! Proving ground (ADRs 0052, 0054, 0055 and 0094, issue 249):
//! - Adding a Group member while a Partitioned crawl runs publishes a higher
//!   partition-map epoch and hands off only the domains won by the new member,
//!   while the Proving ground's observations show no domain fetched by two
//!   Partitions at once.
//! - Removing a Group member (`group leave`) hands off all its domains, and a
//!   Partition left with no domains ends `finished` with Completeness counting
//!   only the domains it still owned.
//! - `runs adopt` completes a Partition handoff from a removed member's kept
//!   volume, carrying pending URLs, seen fingerprints, Wall allowance refetch
//!   stops (ADR 0052), and attached per-domain state to the current owner
//!   Partition rather than resuming the removed Partition as a standalone Run.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test starts Foxhound Daemons, inspects SQLite state, and reads Proving ground observations"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::{DaemonClient, Hello, HomeLayout};
use goodfetch_test_support::{
    ProvingGround, ReceivedRequests, RuntimeFile, TestEnv, cli as command_line,
};
use rusqlite::Connection;
use serde_json::{Value, json};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// Three Group member names whose SHA-256 rendezvous hash distributes
/// `catalog.test`, `extraction.test`, and `parameters.test` across all three
/// members (`member-2`, `member-3`, `member-4`).
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
    assert_eq!(hello["result"]["product"], "foxhound", "{hello}");
    client
}

fn resolving_settings(proving_ground: &ProvingGround, domain_delay_ms: u64) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({
        "fetch": { "tier": "http" },
        "frontier": { "learning": false },
        "network": { "resolve": resolve },
        "egress": {
            "private_ranges": resolve.keys().collect::<Vec<_>>()
        },
        "politeness": {
            "domain": {
                "concurrency": 1,
                "delay": domain_delay_ms,
            }
        },
        "crawl": { "handler": { "concurrency": 2 } }
    })
}

fn join_member(
    lead: &TestEnv,
    worker: &TestEnv,
    peer_address: &str,
    authority_key: &str,
    name: &str,
) {
    let minted = cli_json(lead, &["group", "token"], "JoinTokenMinted");
    let joined = cli_json(
        worker,
        &[
            "group",
            "join",
            "--lead",
            peer_address,
            "--token",
            minted["token"].as_str().unwrap(),
            "--authority-key",
            authority_key,
            "--name",
            name,
            "--peer-address",
            "127.0.0.1:0",
        ],
        "GroupJoined",
    );
    assert!(joined["certificate_expires_at"].is_string(), "{joined}");
    wait_for_replicated_policy(worker);
}

struct TwoMemberGroup {
    peer_address: String,
    authority_key: String,
    lead_client: DaemonClient,
}

fn setup_two_member_group(
    lead: &TestEnv,
    worker_a: &TestEnv,
    extra_daemons: &[&TestEnv],
    settings: &Value,
) -> TwoMemberGroup {
    configure_member_home(lead, true);
    configure_member_home(worker_a, false);
    for env in extra_daemons {
        configure_member_home(env, false);
    }
    for env in std::iter::once(lead)
        .chain(std::iter::once(worker_a))
        .chain(extra_daemons.iter().copied())
    {
        let status = cli_json(env, &["daemon", "start"], "DaemonStatus");
        assert!(status["pid"].is_number(), "{status}");
    }
    let formed = cli_json(
        lead,
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
    join_member(lead, worker_a, &peer_address, &authority_key, WORKER_A_NAME);

    let mut lead_client = open_daemon(lead);
    let written = lead_client.call(
        1,
        "config.user_write",
        &json!({
            "changes": [
                { "key": "network.resolve", "value": settings["network"]["resolve"] },
                { "key": "egress.private_ranges", "value": settings["egress"]["private_ranges"] },
                { "key": "fetch.tier", "value": "http" }
            ]
        }),
    );
    assert!(written.get("error").is_none(), "{written}");
    wait_for_replicated_user_setting(worker_a, "network.resolve");
    std::thread::sleep(Duration::from_millis(250));
    TwoMemberGroup {
        peer_address,
        authority_key,
        lead_client,
    }
}

fn start_two_partition_crawl(
    lead: &TestEnv,
    proving_ground: &ProvingGround,
    crawl_id: &str,
    settings: &Value,
    settings_filename: &str,
) -> Value {
    let catalog_url = proving_ground.start_url("catalog/1").unwrap();
    let extraction_url = proving_ground.start_url("extraction/1").unwrap();
    let parameters_url = proving_ground.start_url("parameters/1").unwrap();
    let settings_path = lead.folder().join(settings_filename);
    std::fs::write(&settings_path, settings.to_string()).unwrap();
    cli_json(
        lead,
        &[
            "crawl",
            "start",
            &catalog_url,
            &extraction_url,
            &parameters_url,
            "--run-id",
            crawl_id,
            "--partitions",
            "2",
            "--format",
            "metadata",
            "--links",
            "scope",
            "--settings",
            settings_path.to_str().unwrap(),
        ],
        "RunStarted",
    )
}

/// Starts a 2-member Foxhound Daemon group crawling 3 Proving ground sites,
/// adds a 3rd Group member mid-crawl (`member-4`) then removes a member
/// (`member-3` via `group leave`), and proves via Proving ground observations
/// that:
/// - no domain was fetched by two Partitions concurrently (zero overlapping
///   request intervals per host),
/// - the crawl finishes cleanly and the removed member's Partition ends
///   `finished` (never `stopped` or `incomplete`), and
/// - all three sites produce their Dataset records.
///
/// Covers: story:3-118
#[test]
fn membership_changes_hand_off_domains_fenced_with_no_concurrent_fetches() {
    let lead = TestEnv::new();
    let worker_a = TestEnv::new();
    let worker_b = TestEnv::new();
    let _stop_lead = StopsDaemon(&lead);
    let _stop_a = StopsDaemon(&worker_a);
    let _stop_b = StopsDaemon(&worker_b);

    let proving_ground = ProvingGround::start(&lead, lead.seed()).expect("Proving ground starts");
    let settings = resolving_settings(&proving_ground, 50);
    let mut group = setup_two_member_group(&lead, &worker_a, &[&worker_b], &settings);

    let crawl_id = "handoff-live-crawl";
    let started = start_two_partition_crawl(
        &lead,
        &proving_ground,
        crawl_id,
        &settings,
        "live-settings.json",
    );
    assert_eq!(
        started["partitions"].as_array().map(Vec::len),
        Some(2),
        "{started}"
    );

    // Wait briefly so both initial Partitions are actively fetching, then add
    // Worker B mid-crawl (triggers map_epoch 2 + fenced handoff of won domains).
    std::thread::sleep(Duration::from_millis(250));
    join_member(
        &lead,
        &worker_b,
        &group.peer_address,
        &group.authority_key,
        WORKER_B_NAME,
    );
    wait_for_replicated_user_setting(&worker_b, "network.resolve");

    // Wait briefly, then remove Worker A via `group leave` (triggers map_epoch
    // 3 + fenced handoff of Worker A's domains to Lead / Worker B).
    std::thread::sleep(Duration::from_millis(300));
    let left = cli_json(&worker_a, &["group", "leave"], "GroupLeft");
    assert_eq!(left["name"], WORKER_A_NAME, "{left}");

    // Wait for the Partitioned crawl to complete and verify its report.
    let report = group
        .lead_client
        .call(3, "run.report", &json!({ "run_id": crawl_id }));
    assert!(report.get("error").is_none(), "{report}");
    assert_eq!(
        report["result"]["lifecycle"], "finished",
        "Partitioned crawl must finish cleanly after membership changes: {report}"
    );

    assert_no_concurrent_fetches(
        &proving_ground,
        &["1.catalog.test", "1.extraction.test", "1.parameters.test"],
    );

    let worker_a_run_file = HomeLayout::of(worker_a.home()).run_file(&format!("{crawl_id}-p1"));
    if worker_a_run_file.is_file() {
        let conn = Connection::open(&worker_a_run_file).unwrap();
        let pending_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM work_items WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            pending_left, 0,
            "removed member's Partition must have handed off all pending work items"
        );
    }
}

fn assert_no_concurrent_fetches(proving_ground: &ProvingGround, hosts: &[&str]) {
    let observations = proving_ground.observations().unwrap();
    let requests = ReceivedRequests::of(&observations);
    let mut by_host: BTreeMap<String, Vec<(u64, u64, String)>> = BTreeMap::new();
    for req in requests.of_the_case() {
        let arrived = req.timing.arrived_microseconds;
        let answered = req.timing.answered_microseconds.unwrap_or(arrived);
        by_host
            .entry(req.host.clone())
            .or_default()
            .push((arrived, answered, req.target.clone()));
    }
    for host in hosts {
        let host_reqs = by_host
            .get(*host)
            .unwrap_or_else(|| panic!("expected observations for {host}, got {by_host:?}"));
        assert!(!host_reqs.is_empty(), "host {host} must have fetched pages");
        let mut sorted = host_reqs.clone();
        sorted.sort_by_key(|(arrived, _, _)| *arrived);
        for window in sorted.windows(2) {
            let (prev_start, prev_end, ref prev_target) = window[0];
            let (next_start, _, ref next_target) = window[1];
            assert!(
                next_start >= prev_end,
                "concurrent fetch overlap on {host}: {prev_target} [{prev_start}..{prev_end}] overlapped {next_target} at {next_start}"
            );
        }
    }
}

/// Offline `runs adopt` completes a Partition handoff from a removed Group
/// member's kept volume, handing its remaining domains (pending URLs, seen
/// fingerprints, Wall allowance refetch stop in `run-index.sqlite`, and
/// attached `partition_domain_state`) to their current owner Partition rather
/// than resuming the removed Partition as a standalone Run.
///
/// Covers: story:3-118
#[test]
fn runs_adopt_completes_partition_handoff_and_carries_refetch_stop_and_domain_state() {
    let lead = TestEnv::new();
    let worker_a = TestEnv::new();
    let _stop_lead = StopsDaemon(&lead);
    let _stop_a = StopsDaemon(&worker_a);

    let proving_ground = ProvingGround::start(&lead, lead.seed()).expect("Proving ground starts");
    let settings = resolving_settings(&proving_ground, 40);
    let mut group = setup_two_member_group(&lead, &worker_a, &[], &settings);

    // Use `crawl.keep_alive = true` on Lead's Partition so Lead stays live to
    // receive `runs adopt` handoffs from Worker A's kept volume!
    let mut keep_alive_settings = settings.clone();
    keep_alive_settings["crawl"]["keep_alive"] = json!(true);

    let started = start_two_partition_crawl(
        &lead,
        &proving_ground,
        "handoff-adopt-crawl",
        &keep_alive_settings,
        "adopt-settings.json",
    );

    let parts = started["partitions"].as_array().unwrap();
    let lead_part_id = parts.iter().find(|p| p["member"] == LEAD_NAME).unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let worker_part_id = parts.iter().find(|p| p["member"] == WORKER_A_NAME).unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let worker_run_folder = HomeLayout::of(worker_a.home()).run_folder(&worker_part_id);
    let worker_run_file = worker_run_folder.join("run.sqlite");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !worker_run_file.is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        worker_run_file.is_file(),
        "Worker A Partition run file must exist"
    );

    let _ = command_line(&worker_a, FOXHOUND)
        .args(["daemon", "stop"])
        .run();

    let adopt_domain = "1.catalog.test";
    let adopt_url = proving_ground.url(adopt_domain, "/adopted-check");
    plant_worker_state_for_adopt(
        &worker_a,
        &worker_run_file,
        &worker_part_id,
        adopt_domain,
        &adopt_url,
    );

    let _ = cli_json(
        &lead,
        &["group", "revoke", "--name", WORKER_A_NAME],
        "GroupMemberRevoked",
    );
    let adopted = cli_json(
        &lead,
        &[
            "runs",
            "adopt",
            worker_run_folder.to_str().unwrap(),
            "--from-member",
            WORKER_A_NAME,
        ],
        "RunsAdoptOutput",
    );
    let adopted_runs = adopted["adopted"].as_array().unwrap();
    assert!(
        adopted_runs
            .iter()
            .any(|entry| entry["run_id"] == lead_part_id && entry["owner"] == LEAD_NAME),
        "runs adopt must hand Worker A's remaining domains to Lead's live Partition (not resume Worker A's Partition): {adopted}"
    );

    assert_adopt_state_carried(
        &lead,
        &mut group.lead_client,
        &proving_ground,
        &lead_part_id,
        adopt_domain,
    );
}

fn plant_worker_state_for_adopt(
    worker_a: &TestEnv,
    worker_run_file: &std::path::Path,
    worker_part_id: &str,
    adopt_domain: &str,
    adopt_url: &str,
) {
    let conn = Connection::open(worker_run_file).unwrap();
    let next_pos: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM work_items",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let existing_label: String = conn
        .query_row("SELECT label FROM work_items LIMIT 1", [], |row| row.get(0))
        .unwrap_or_else(|_| "default".to_owned());
    conn.execute(
        "INSERT INTO work_items
            (id, position, url, method, body, label, depth, status, attempts, retried_from, lease_epoch, priority, forefront, domain_key)
         VALUES (?1, ?2, ?3, 'GET', NULL, ?4, 0, 'pending', 0, 0, 0, 0, NULL, ?5)",
        (
            format!("adopt-{next_pos}"),
            next_pos,
            adopt_url,
            existing_label.as_str(),
            adopt_domain,
        ),
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO partition_domain_state (domain_key, area, state_json)
         VALUES (?1, 'wall_allowance', ?2)",
        (
            adopt_domain,
            r#"{"refetch_stopped":true,"note":"carried_by_adopt"}"#,
        ),
    )
    .unwrap();

    let worker_index = HomeLayout::of(worker_a.home())
        .per_major()
        .join("run-index.sqlite");
    let idx_conn = Connection::open(&worker_index).unwrap();
    idx_conn
        .execute(
            "INSERT OR REPLACE INTO refetch_stops (domain, run_id, cause, project_file, stopped_at)
             VALUES (?1, ?2, 'challenge', NULL, '2026-11-28T00:00:00Z')",
            (adopt_domain, worker_part_id),
        )
        .unwrap();
}

fn assert_adopt_state_carried(
    lead: &TestEnv,
    lead_client: &mut DaemonClient,
    proving_ground: &ProvingGround,
    lead_part_id: &str,
    adopt_domain: &str,
) {
    let lead_index = HomeLayout::of(lead.home())
        .per_major()
        .join("run-index.sqlite");
    let lead_stops: BTreeSet<String> = {
        let conn = Connection::open(&lead_index).unwrap();
        let mut stmt = conn
            .prepare("SELECT domain FROM refetch_stops WHERE run_id = ?1")
            .unwrap();
        stmt.query_map([lead_part_id], |row| row.get::<_, String>(0))
            .unwrap()
            .flatten()
            .collect()
    };
    assert!(
        lead_stops.contains(adopt_domain),
        "Wall allowance refetch stop for `{adopt_domain}` must be carried into Lead's Run index by `runs adopt`; got {lead_stops:?}"
    );

    let lead_run_file = HomeLayout::of(lead.home()).run_file(lead_part_id);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut fetched_adopted = false;
    while Instant::now() < deadline {
        let obs = proving_ground.observations().unwrap();
        if ReceivedRequests::of(&obs)
            .of_the_case()
            .any(|r| r.host == adopt_domain && r.target == "/adopted-check")
        {
            fetched_adopted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        fetched_adopted,
        "Lead's Partition must admit and fetch the pending URL handed off by `runs adopt`"
    );

    let _ = lead_client.call(3, "run.finish", &json!({ "run_id": lead_part_id }));
    let _ = lead_client.call(4, "run.report", &json!({ "run_id": lead_part_id }));

    let carried_state: Option<String> = {
        let conn = Connection::open(&lead_run_file).unwrap();
        conn.query_row(
            "SELECT state_json FROM partition_domain_state WHERE domain_key = ?1 AND area = 'wall_allowance'",
            [adopt_domain],
            |row| row.get(0),
        )
        .ok()
    };
    assert!(
        carried_state
            .as_deref()
            .is_some_and(|s| s.contains("carried_by_adopt")),
        "attached per-domain state must be carried by `runs adopt`; got {carried_state:?}"
    );
}
