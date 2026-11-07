//! J-015, "Run the Daemon in the foreground, drain it with `SIGTERM` and
//! resume its runs when it starts again", on the `cli` surface (ADRs 0049,
//! 0063 and 0075): a person runs Collie's shipped executable as `daemon
//! run`, configured by a user configuration file and an environment
//! variable, and a program of theirs, a Client speaking the protocol, runs
//! two runs on it, each visiting its share of the Proving ground's catalog
//! in a browser, across a `SIGTERM` and a second start.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps start, signal and wait for the Daemon, and wait past its idle time"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, DaemonEndpoint, JourneyRun, ProvingGround, RuntimeFile, browser_cache,
};
use serde_json::{Map, Value, json};

use crate::daemon::RawClient;
use crate::daemon_run::{ANY_PORT, Foreground, opened, took_a_free_port};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version, whose major folder is `0.0`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The two runs, each with its share of the catalog's product numbers.
const SHARES: [(&str, [u32; 6]); 2] = [
    ("first-share", [1, 2, 3, 4, 5, 6]),
    ("second-share", [7, 8, 9, 10, 11, 12]),
];

/// How many pages of its share each run visits before the `SIGTERM`.
const BEFORE: usize = 3;

/// The drain grace the environment variable gives, in milliseconds.
const DRAIN_GRACE: &str = "20000";

/// The run settings that map the Proving ground's hosts to its address.
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

/// Opens a page in `run_id`'s default Session and visits `numbers`'
/// product pages one after another, each answered with its document.
fn visit(
    client: &mut RawClient,
    next_id: &mut i64,
    run_id: &str,
    numbers: &[u32],
    proving_ground: &ProvingGround,
) {
    *next_id += 1;
    let page = client.call(*next_id, "run.new_page", &json!({ "run_id": run_id }));
    let page_id = page["result"]["page_id"].clone();
    assert!(!page_id.is_null(), "{page}");
    for number in numbers {
        *next_id += 1;
        let url = proving_ground.url("catalog.test", &format!("/products/P-{number:04}"));
        let went = client.call(
            *next_id,
            "page.goto",
            &json!({ "page_id": page_id, "url": url }),
        );
        assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    }
}

/// How many times the Proving ground served each product page.
fn served(proving_ground: &ProvingGround) -> BTreeMap<String, usize> {
    let mut served = BTreeMap::new();
    for request in proving_ground.received_requests().unwrap().of_the_case() {
        if let Some(path) = request["target"]
            .as_str()
            .filter(|path| request["host"] == "catalog.test" && path.starts_with("/products/"))
        {
            *served.entry(path.to_owned()).or_insert(0) += 1;
        }
    }
    served
}

/// Covers: story:3-98, story:3-99
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-015", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-015's cli variant starts");
    let cache = browser_cache(run.env()).expect("Playwright's Chrome for Testing is installed");
    let cache = cache.to_string_lossy().into_owned();
    let mut variables: Vec<(&str, &str)> = vec![(BROWSERS_PATH_VARIABLE, &cache)];
    let mut first: Option<Foreground> = None;
    let mut endpoint: Option<DaemonEndpoint> = None;
    let mut client: Option<RawClient> = None;
    let mut next_id = 0;
    let mut still_serving = Value::Null;
    let mut late_start: Option<Value> = None;
    let mut drained_exit = false;
    let mut resumed = Vec::new();
    let mut second_exit = false;
    let mut second_logged = Vec::new();

    run.step(
        "Configure the Daemon's address through a configuration file and its drain grace period through an environment variable",
        |run| {
            run.env().user_configuration(&format!(
                "daemon.address = \"{ANY_PORT}\"\ndaemon.idle_timeout = 100\n"
            ));
            variables.push(("GOODFETCH_DAEMON_DRAIN_GRACE", DRAIN_GRACE));
        },
    );
    run.step("Run the Daemon in the foreground (`daemon run`)", |run| {
        let mut daemon = Foreground::start(run.env(), run.engine().to_str().unwrap(), &variables);
        endpoint = Some(daemon.serving());
        first = Some(daemon);
    });
    run.step(
        "Wait past the Daemon's idle time with no Client connected",
        |run| {
            std::thread::sleep(Duration::from_secs(1));
            let endpoint = endpoint.as_ref().expect("the Daemon serves");
            let mut asking = opened(run.env(), endpoint);
            still_serving = asking.call(1, "daemon.status", &json!({}));
        },
    );
    run.step(
        "Start two runs on the Daemon, each visiting its own share of the catalog's product pages one after another",
        |run| {
            let proving_ground = run.proving_ground().expect("J-015 has a Proving ground");
            let mut opening = opened(run.env(), endpoint.as_ref().expect("the Daemon serves"));
            for (run_id, numbers) in SHARES {
                next_id += 1;
                let started = opening.call(
                    next_id,
                    "run.start",
                    &json!({ "run_id": run_id, "settings": resolving(proving_ground) }),
                );
                assert!(started.get("result").is_some(), "{started}");
                visit(&mut opening, &mut next_id, run_id, &numbers[..BEFORE], proving_ground);
            }
            client = Some(opening);
        },
    );
    run.step(
        "Send the Daemon `SIGTERM` while both runs are going on",
        |_| {
            let daemon = first.as_mut().expect("the Daemon runs");
            daemon.signal();
            daemon.record("the Daemon drains: it takes no run and checkpoints each live run");
            // A run asked for now is refused, or meets the Connection closed by
            // a Daemon that already exited.
            let late = client.as_mut().expect("the program is connected");
            next_id += 1;
            late.try_send(&json!({
                "jsonrpc": "2.0",
                "id": next_id,
                "method": "run.start",
                "params": { "run_id": "late" },
            }));
            late_start = Some(late.next().unwrap_or(Value::Null));
            drained_exit = daemon.exited();
            client = None;
        },
    );
    run.step(
        "Run the Daemon in the foreground again and let both runs finish",
        |run| {
            let proving_ground = run.proving_ground().expect("J-015 has a Proving ground");
            let mut daemon =
                Foreground::start(run.env(), run.engine().to_str().unwrap(), &variables);
            let endpoint = daemon.serving();
            let mut resuming = opened(run.env(), &endpoint);
            for (run_id, numbers) in SHARES {
                next_id += 1;
                let again = resuming.call(
                    next_id,
                    "run.resume",
                    &json!({ "run_id": run_id, "settings": resolving(proving_ground) }),
                );
                visit(
                    &mut resuming,
                    &mut next_id,
                    run_id,
                    &numbers[BEFORE..],
                    proving_ground,
                );
                next_id += 1;
                let finished = resuming.call(next_id, "run.finish", &json!({ "run_id": run_id }));
                resumed.push((
                    again["result"]["run_id"].clone(),
                    again["result"]["started_as"].clone(),
                    finished["result"]["lifecycle"].clone(),
                ));
            }
            drop(resuming);
            second_exit = daemon.terminate();
            second_logged = daemon.logged.clone();
        },
    );

    let home = run.env().home().to_owned();
    let first_logged = first
        .as_ref()
        .map(|daemon| daemon.logged.clone())
        .unwrap_or_default();
    run.check(
        "The foreground Daemon listened on the configured address and wrote no runtime file",
        |_| {
            assert_eq!(
                (
                    endpoint.as_ref().is_some_and(took_a_free_port),
                    RuntimeFile::path(&home, VERSION).exists()
                ),
                (true, false)
            );
        },
    );
    run.check("The foreground Daemon logged JSON lines to stdout", |_| {
        let not_json: Vec<&String> = first_logged
            .iter()
            .chain(&second_logged)
            .filter(|line| serde_json::from_str::<Value>(line).is_err())
            .collect();
        assert_eq!(
            (first_logged.is_empty(), not_json),
            (false, Vec::<&String>::new())
        );
    });
    run.check(
        "The foreground Daemon was still running after its idle time with no Client connected",
        |_| {
            assert_eq!(
                still_serving["result"]["kept_alive"],
                json!(true),
                "{still_serving}"
            );
        },
    );
    run.check(
        "On `SIGTERM` the Daemon refused new runs, checkpointed both runs as resumable and exited within its drain grace period",
        |_| {
            let runs = HomeLayout::of(&home).runs();
            let late = late_start.clone().unwrap_or(Value::Null);
            let refused = late.is_null() || late["error"]["data"]["code"] == json!("daemon_draining");
            let checkpointed = SHARES.map(|(run_id, _)| runs.join(run_id).join("checkpoint.sqlite").is_file());
            assert_eq!(
                (refused, runs.join("late").exists(), checkpointed, drained_exit),
                (true, false, [true, true], true),
                "{late}"
            );
        },
    );
    run.check(
        "When the Daemon started again, both runs resumed by their Run IDs and finished, and the catalog served every product page of both shares once, none lost and none twice",
        |run| {
            let served = served(run.proving_ground().expect("J-015 has a Proving ground"));
            let every_page_once: BTreeMap<String, usize> = SHARES
                .iter()
                .flat_map(|(_, numbers)| numbers.iter())
                .map(|number| (format!("/products/P-{number:04}"), 1))
                .collect();
            assert_eq!(
                (resumed.clone(), served, second_exit),
                (
                    SHARES
                        .map(|(run_id, _)| (json!(run_id), json!("resumed"), json!("finished")))
                        .to_vec(),
                    every_page_once,
                    true,
                )
            );
        },
    );
    run.finish()
        .expect("J-015's document is written")
        .assert_proved();
}
