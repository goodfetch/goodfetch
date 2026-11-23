//! The foreground Daemon's ops listener and metrics, an Attached engine's
//! at `ops.attached_address`, and `daemon health`, through Collie's
//! executable (spec #213's Seam 1; ADR 0050): the ops
//! listener passes its column of the endpoint table; its probes answer
//! whether the Daemon is live and ready with a status alone; `/metrics`
//! answers the ops token with the runtime's metrics, each label taking a
//! value the protocol declares; the metrics reach an OTLP collector once
//! one is configured; and `daemon health` answers the probes' questions
//! for `exec` probes and systemd.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Daemon's files and serve the collector stand-in from their own threads"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the collector stand-in listens outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Metric;
use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    Canaries, DaemonEndpoint, Endpoint, EndpointRunner, Listener, ListenerKind, ProvingGround,
    TestEnv, Variables, browser_cache, cli,
};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use prost::Message as _;
use rstest::rstest;
use serde_json::{Value, json};

use crate::daemon::validate;
use crate::daemon_run::{ANY_PORT, Foreground, opened, token};
use crate::endpoint::{audit_verifier, provision_ops_token};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// How long a test waits for the collector to receive an export; a
/// failure, never a measure.
const EXPORT_DEADLINE: Duration = Duration::from_secs(60);

/// The ops token the Daemon of `env` made.
fn ops_token(env: &TestEnv) -> String {
    std::fs::read_to_string(HomeLayout::of(env.home()).daemon_file(DaemonFile::OpsToken)).unwrap()
}

/// Whether the Browser cache holds the pinned build, or is an empty one.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Browsers {
    Installed,
    Missing,
}

/// The `GOODFETCH_BROWSERS_PATH` of `browsers` in `env`.
fn browsers_path(env: &TestEnv, browsers: Browsers) -> PathBuf {
    match browsers {
        Browsers::Installed => browser_cache(env).unwrap(),
        Browsers::Missing => {
            let empty = env.folder().join("no browsers");
            std::fs::create_dir_all(&empty).unwrap();
            empty
        }
    }
}

/// A foreground Daemon with an ops listener on a free port, its configured
/// browsers `browsers`; and where its ops listener listens.
pub(crate) fn with_ops(env: &TestEnv, browsers: Browsers) -> (Foreground, SocketAddr) {
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\nops.address = \"127.0.0.1:0\"\n"
    ));
    let path = browsers_path(env, browsers);
    let mut daemon = Foreground::start(
        env,
        COLLIE,
        &[("GOODFETCH_BROWSERS_PATH", path.to_str().unwrap())],
    );
    let ops = daemon.record("the ops listener serves")["address"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    (daemon, ops)
}

/// The status and body of `GET path` at `address`, with `token` as a
/// bearer token when given.
pub(crate) fn get(address: SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut request = agent.get(format!("http://{address}{path}"));
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let mut answer = request.call().unwrap();
    let status = answer.status().as_u16();
    (status, answer.body_mut().read_to_string().unwrap())
}

/// The endpoint table's ops listener column, each refusal found in the
/// Audit log: the ops token the operator replaced before this launch is
/// revoked, and the Daemon's own protocol token, which has no read scope on
/// `/metrics`, is both another listener's and scoped elsewhere.
///
/// Covers: story:3-101, story:13-44, command:daemon.run, setting:ops.address, story:4-21, story:4-24, story:4-29, story:4-90
#[test]
fn the_ops_listener_passes_its_endpoint_table_rows() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let revoked = provision_ops_token(&env, &mut canaries);
    let (mut earlier, _) = with_ops(&env, Browsers::Installed);
    earlier.terminate();
    let current = provision_ops_token(&env, &mut canaries);
    let (mut daemon, address) = with_ops(&env, Browsers::Installed);
    let listener = Listener::new(
        ListenerKind::OpsListener,
        Endpoint::Http { address },
        current,
    )
    .with_other_listener_token(token(&env))
    .with_revoked_token(revoked)
    .with_scoped_elsewhere_token(token(&env));

    let ran = EndpointRunner::new(audit_verifier(&env, "ops_listener")).run(&listener);

    daemon.terminate();
    assert_eq!(ran, Ok(()));
}

/// `/livez` and `/readyz` answer anyone with a status and no body: live
/// while the Daemon runs, ready only while the browser its configuration
/// uses is installed.
///
/// Covers: story:3-101, story:3-102, story:13-44
#[rstest]
#[case::browsers_installed(Browsers::Installed, (200, 200))]
#[case::browsers_missing(Browsers::Missing, (200, 503))]
fn the_probes_answer_whether_the_daemon_is_live_and_ready(
    #[case] browsers: Browsers,
    #[case] statuses: (u16, u16),
) {
    let env = TestEnv::new();
    let (mut daemon, address) = with_ops(&env, browsers);

    let live = get(address, "/livez", None);
    let ready = get(address, "/readyz", None);

    daemon.terminate();
    assert_eq!(
        (live, ready),
        ((statuses.0, String::new()), (statuses.1, String::new()))
    );
}

/// The protocol's metric of the Prometheus series `name`: its name with
/// `.` rendered `_`, and a counter's `_total`, a ratio's `_ratio`, a size's
/// `_bytes`, or a histogram's `_bucket`, `_sum` or `_count`.
fn declared(name: &str) -> Option<&'static Metric> {
    MODULES
        .iter()
        .flat_map(|entries| entries.metrics)
        .find(|metric| {
            let rendered = metric.name.replace('.', "_");
            [
                rendered.clone(),
                format!("{rendered}_total"),
                format!("{rendered}_ratio"),
                format!("{rendered}_bytes"),
                format!("{rendered}_bucket"),
                format!("{rendered}_sum"),
                format!("{rendered}_count"),
            ]
            .contains(&name.to_owned())
        })
}

/// Whether the label `key` of the Prometheus series `name` is the bucket
/// bound Prometheus gives each `_bucket` series of a histogram.
fn bucket_bound(name: &str, key: &str) -> bool {
    key == "le" && name.ends_with("_bucket")
}

/// A series of a Prometheus text: its name, its labels and its value.
pub(crate) type Series = (String, BTreeMap<String, String>, String);

/// Each series of a Prometheus text.
pub(crate) fn series(text: &str) -> Vec<Series> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let (series, value) = line.rsplit_once(' ').unwrap();
            let (name, labels) = match series.split_once('{') {
                Some((name, labels)) => (name, labels.trim_end_matches('}')),
                None => (series, ""),
            };
            let labels = labels
                .split(',')
                .filter(|label| !label.is_empty())
                .map(|label| {
                    let (key, value) = label.split_once('=').unwrap();
                    (key.to_owned(), value.trim_matches('"').to_owned())
                })
                .collect();
            (name.to_owned(), labels, value.to_owned())
        })
        .collect()
}

/// `/metrics` answers the ops token with Prometheus' text of the runtime's
/// metrics: a Run that finished, a Run still live with its page's browser
/// running, and the Client that started them; each series a metric the
/// protocol declares, each label one of its declared values or a
/// histogram's bucket bound.
///
/// Covers: story:3-101, story:3-103
#[test]
fn metrics_answer_the_ops_token_with_the_runtimes_metrics_under_bounded_labels() {
    let env = TestEnv::new();
    let (mut daemon, address) = with_ops(&env, Browsers::Installed);
    let endpoint = daemon.serving();
    let mut client = opened(&env, &endpoint);
    client.call(1, "run.start", &json!({ "run_id": "finished" }));
    client.call(2, "run.finish", &json!({ "run_id": "finished" }));
    client.call(3, "run.start", &json!({ "run_id": "browsing" }));
    let page = client.call(4, "run.new_page", &json!({ "run_id": "browsing" }));
    assert!(page.get("result").is_some(), "{page}");

    let (status, text) = get(address, "/metrics", Some(&ops_token(&env)));

    drop(client);
    daemon.terminate();
    let undeclared: Vec<String> = series(&text)
        .iter()
        .filter(|(name, labels, _)| {
            declared(name).is_none_or(|metric| {
                labels.iter().any(|(key, value)| {
                    !bucket_bound(name, key)
                        && !metric.labels.iter().any(|label| {
                            label.name == key && (label.values)().contains(&value.as_str())
                        })
                })
            })
        })
        .map(|(name, labels, _)| format!("{name}{labels:?}"))
        .collect();
    let lines: BTreeSet<&str> = text.lines().collect();
    // Where the tests run with no GUI session, Runs launch the headless
    // shell (#3224).
    let running = if Variables::from_process().no_gui_session() {
        r#"goodfetch_browser_running{browser="chrome_headless_shell"} 1"#
    } else {
        r#"goodfetch_browser_running{browser="chrome_for_testing"} 1"#
    };
    assert_eq!(
        (
            status,
            undeclared,
            [
                running,
                r#"goodfetch_client_connected{surface="sdk_typescript"} 1"#,
                "goodfetch_daemon_draining 0",
                r#"goodfetch_run_active{run_kind="collie"} 1"#,
                r#"goodfetch_run_ended_total{completeness="complete",lifecycle="finished",run_kind="collie",verdict="verified"} 1"#,
            ]
            .map(|line| lines.contains(line)),
        ),
        (200, Vec::<String>::new(), [true; 5])
    );
}

/// The value of the series `name` with no label in the Prometheus text
/// `/metrics` at `address` answers `token`, if it has one.
fn unlabelled(address: SocketAddr, token: &str, name: &str) -> Option<f64> {
    let (_, text) = get(address, "/metrics", Some(token));
    series(&text)
        .into_iter()
        .find(|(series, labels, _)| series == name && labels.is_empty())
        .map(|(_, _, value)| value.parse().unwrap())
}

/// A Run whose browser opened its `governor.browser.recycle_pages` pages
/// has it recycled at its next page, which `goodfetch.browser.recycled`
/// counts with the reason `pages`; and once the Resource governor measured
/// the Run's browser, `goodfetch.browser.memory` holds what it uses.
///
/// Covers: story:3-103, story:3-62, setting:governor.browser.recycle_pages
#[test]
fn metrics_count_each_browser_recycled_and_what_the_browsers_use() {
    let env = TestEnv::new();
    let (mut daemon, address) = with_ops(&env, Browsers::Installed);
    let endpoint = daemon.serving();
    let mut client = opened(&env, &endpoint);
    let token = ops_token(&env);
    let settings = json!({ "governor": { "browser": { "recycle_pages": 1 } } });
    client.call(
        1,
        "run.start",
        &json!({ "run_id": "recycling", "settings": settings }),
    );
    let first = client.call(2, "run.new_page", &json!({ "run_id": "recycling" }));
    let deadline = Instant::now() + SAMPLE_DEADLINE;
    let memory = loop {
        let memory = unlabelled(address, &token, "goodfetch_browser_memory_bytes");
        if memory.is_some_and(|memory| memory > 0.0) || Instant::now() >= deadline {
            break memory;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let second = client.call(3, "run.new_page", &json!({ "run_id": "recycling" }));

    let (_, text) = get(address, "/metrics", Some(&token));
    drop(client);
    daemon.terminate();
    let lines: BTreeSet<&str> = text.lines().collect();
    assert_eq!(
        (
            first.get("result").is_some(),
            second.get("result").is_some(),
            memory.is_some_and(|memory| memory > 0.0),
            lines.contains(r#"goodfetch_browser_recycled_total{reason="pages"} 1"#),
        ),
        (true, true, true, true),
        "{first}\n{second}\n{memory:?}\n{text}"
    );
}

/// A collector stand-in on loopback: it hands each OTLP/HTTP request's
/// path and body to the test, then answers it with 200, so an export the
/// Daemon saw answered is already the test's.
fn collector() -> (SocketAddr, mpsc::Receiver<(String, Vec<u8>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sent, received) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let path = request_line
                .split(' ')
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            if sent.send((path, body)).is_err() {
                return;
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .unwrap();
        }
    });
    (address, received)
}

/// The metrics a Collie Daemon has a series of once a Client ran a Run and
/// the Resource governor sampled the machine and the filesystems of the
/// Daemon's own folders (ADRs 0050 and 0122): `goodfetch.browser.running`
/// and `goodfetch.browser.recycled` have none, since no browser ran,
/// `goodfetch.browser.memory` is 0, `goodfetch.run_state.wal.size` holds
/// the Run's run file's write-ahead log as of its last commit (ADR 0138),
/// `goodfetch.run_state.batch_rows` the rows of the run file's batches
/// (ADR 0142), `goodfetch.run_state.blocked_checkpoints` has none, since no
/// reader blocked a checkpoint, `goodfetch.daemon.heap` holds the live heap
/// the executable's allocator counts (#1769), and [`INODES`] joins them
/// only where the filesystem reports its inodes.
const SAMPLED: [&str; 14] = [
    "goodfetch.browser.memory",
    "goodfetch.client.connected",
    "goodfetch.counted_work",
    "goodfetch.daemon.draining",
    "goodfetch.daemon.heap",
    "goodfetch.governor.pressure_source",
    "goodfetch.governor.saturation",
    "goodfetch.governor.target",
    "goodfetch.run.active",
    "goodfetch.run.ended",
    "goodfetch.run_state.batch_rows",
    "goodfetch.run_state.wal.size",
    "goodfetch.storage.available",
    "goodfetch.storage.level",
];

/// The inodes' metric, which only a filesystem that reports its inodes
/// gives a series.
const INODES: &str = "goodfetch.storage.inodes.available";

/// How long a test waits for the governor's samples to reach the metrics;
/// a failure, never a measure.
const SAMPLE_DEADLINE: Duration = Duration::from_secs(30);

/// The protocol's name of each metric `/metrics` at `address` serves a
/// series of.
fn scraped(address: SocketAddr, token: &str) -> BTreeSet<&'static str> {
    let (_, text) = get(address, "/metrics", Some(token));
    series(&text)
        .iter()
        .filter_map(|(name, _, _)| declared(name))
        .map(|metric| metric.name)
        .collect()
}

/// What [`scraped`] reads once it holds each of [`SAMPLED`]: the
/// governor samples on its own clock, and its first sample may come
/// before the Daemon's folders are held, so their filesystems' series wait
/// for a later one. Fails the test past [`SAMPLE_DEADLINE`] with the last
/// read.
fn sampled(address: SocketAddr, token: &str) -> BTreeSet<&'static str> {
    let deadline = Instant::now() + SAMPLE_DEADLINE;
    loop {
        let read = scraped(address, token);
        if SAMPLED.iter().all(|name| read.contains(name)) {
            return read;
        }
        assert!(
            Instant::now() < deadline,
            "the metrics never held each sampled metric: {read:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// With `otlp.metrics_endpoint` set, the foreground Daemon exports its
/// metrics there over OTLP/HTTP in protobuf, once more as it exits on
/// `SIGTERM`: its last export holds every metric it had a series of. Its
/// ops listener tells the test when the governor's samples reached them.
///
/// Covers: story:3-103, setting:otlp.metrics_endpoint
#[test]
fn with_otlp_metrics_endpoint_set_the_collector_receives_the_runtimes_metrics() {
    let env = TestEnv::new();
    let (collector, exports) = collector();
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\nops.address = \"127.0.0.1:0\"\notlp.metrics_endpoint = \"http://{collector}/v1/metrics\"\n"
    ));
    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let ops: SocketAddr = daemon.record("the ops listener serves")["address"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let endpoint = daemon.serving();
    let mut client = opened(&env, &endpoint);
    client.call(1, "run.start", &json!({ "run_id": "exported" }));
    client.call(2, "run.finish", &json!({ "run_id": "exported" }));
    drop(client);
    let sampled = sampled(ops, &ops_token(&env));

    let exited = daemon.terminate();

    let (path, body) = std::iter::once(exports.recv_timeout(EXPORT_DEADLINE).unwrap())
        .chain(exports.try_iter())
        .last()
        .unwrap();
    let request = ExportMetricsServiceRequest::decode(body.as_slice()).unwrap();
    let names: BTreeSet<String> = request
        .resource_metrics
        .iter()
        .flat_map(|resource| &resource.scope_metrics)
        .flat_map(|scope| &scope.metrics)
        .map(|metric| metric.name.clone())
        .collect();
    let expected: BTreeSet<String> = SAMPLED
        .into_iter()
        .chain(sampled.contains(INODES).then_some(INODES))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        (exited, path, names),
        (true, "/v1/metrics".to_owned(), expected)
    );
}

/// `daemon health --json` of the test's home: its exit code and output.
fn health(env: &TestEnv, arguments: &[&str]) -> (Option<i32>, Value) {
    let output = cli(env, COLLIE)
        .args(["daemon", "health", "--json"])
        .args(arguments)
        .run()
        .unwrap();
    let printed: Value = serde_json::from_str(&output.stdout).unwrap();
    validate(&printed, "DaemonHealthOutput");
    (output.exit, printed)
}

/// `daemon health` answers the probes' questions of the foreground Daemon
/// at `daemon.address`, for `exec` probes and systemd: it exits 0 when the
/// answer is yes, and 1 when it is no.
///
/// Covers: story:3-104, command:daemon.health
#[rstest]
#[case::ready(Browsers::Installed, &[], 0, json!({ "live": true, "ready": true, "unready": [] }))]
#[case::not_ready(Browsers::Missing, &[], 1, json!({ "live": true, "ready": false, "unready": ["browsers_missing"] }))]
#[case::live_though_not_ready(Browsers::Missing, &["--live"], 0, json!({ "live": true, "ready": false, "unready": ["browsers_missing"] }))]
fn daemon_health_answers_whether_the_foreground_daemon_is_ready_or_live(
    #[case] browsers: Browsers,
    #[case] arguments: &[&str],
    #[case] exit: i32,
    #[case] answered: Value,
) {
    let env = TestEnv::new();
    let (mut daemon, _) = with_ops(&env, browsers);
    let DaemonEndpoint::Tcp(serving) = daemon.serving() else {
        panic!("`daemon run` listens on TCP");
    };
    // `daemon health` reaches the foreground Daemon at `daemon.address`,
    // which now names the port the Daemon took.
    env.user_configuration(&format!("daemon.address = \"{serving}\"\n"));

    let (code, printed) = health(&env, arguments);

    daemon.terminate();
    let probe = if arguments.is_empty() {
        "ready"
    } else {
        "live"
    };
    assert_eq!(
        (code, printed),
        (
            Some(exit),
            json!({ "probe": probe, "healthy": exit == 0, "daemon": answered })
        )
    );
}

/// With no Daemon of the major running, `daemon health` says so and fails.
///
/// Covers: story:3-104, command:daemon.health
#[test]
fn daemon_health_fails_when_no_daemon_answers() {
    let env = TestEnv::new();
    env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n"));

    let answered = health(&env, &[]);

    assert_eq!(
        answered,
        (
            Some(1),
            json!({ "probe": "ready", "healthy": false, "daemon": null })
        )
    );
}

/// The Proving ground's own seed for these tests.
const SEED: u64 = 732;

/// A port on loopback no listener holds now.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// An Attached engine that runs its Runs itself serves the ops listener
/// `ops.attached_address` names while it runs: `/metrics` answers the ops
/// token alone, with the engine's Counted work among it: the CDP commands
/// it sent for a page it loaded, and the requests it paused, counted on the
/// same connection's Counted work (a load pauses none once the relay
/// enforces egress, the Quality register's X18).
///
/// Covers: setting:ops.attached_address, story:3-101
#[test]
fn an_attached_engine_serves_its_counted_work_at_ops_attached_address() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let address = SocketAddr::from(([127, 0, 0, 1], free_port()));
    env.user_configuration(&format!("ops.attached_address = \"{address}\"\n"));
    let mut engine = crate::page::started(&env, &cache, &proving_ground, "counted");
    let page = crate::page::opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "counted" })));
    let went = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("catalog.test", "/catalog/1") }),
    );

    let refused = get(address, "/metrics", None).0;
    let (status, text) = get(address, "/metrics", Some(&ops_token(&env)));
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "counted" }));

    assert_eq!(engine.finish(), (vec![], true));
    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    assert_eq!((refused, status), (401, 200), "{text}");
    let counted = |path: &str, work: &str| -> Option<u64> {
        series(&text)
            .into_iter()
            .find(|(name, labels, _)| {
                name == "goodfetch_counted_work_total"
                    && labels["shared_path"] == path
                    && labels["work"] == work
            })
            .map(|(_, _, value)| value.parse::<u64>().unwrap())
    };
    let (sends, answers) = (
        counted("cdp_command", "sends"),
        counted("intercepted_request", "answers"),
    );
    assert!(
        sends.is_some_and(|sends| sends > 0) && answers.is_some(),
        "sends {sends:?}, answers {answers:?}: {text}"
    );
}

/// An `ops.attached_address` that is no loopback `host:port` stops the
/// Attached engine before it serves, its Client reading no message, and
/// its log names the refusal's Error code and the key.
///
/// Covers: setting:ops.attached_address
#[test]
fn an_attached_engine_refuses_an_ops_attached_address_off_loopback() {
    let env = TestEnv::new();
    env.user_configuration("ops.attached_address = \"0.0.0.0:9465\"\n");
    let engine = goodfetch_test_client::Attached::start(&env, COLLIE);

    let (sent, stderr) = engine.finish_with_stderr();
    assert_eq!(
        (
            sent,
            stderr.contains("setting_invalid"),
            stderr.contains("ops.attached_address")
        ),
        (vec![], true, true),
        "{stderr}"
    );
}
