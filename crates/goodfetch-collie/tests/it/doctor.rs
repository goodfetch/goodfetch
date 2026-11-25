//! `doctor` through Collie's executable (spec #608, ADR 0043): one finding
//! for each part of the installation, with a fix for each that is not in
//! order, and exit `0` whatever it finds; and the bug-report file it
//! writes and sends nowhere (spec #564).
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the bug-report file `doctor` wrote, outside any async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::doctor::{BugReportFile, DoctorReport, FindingId, FindingStatus};
use goodfetch_protocol::home::BUG_REPORT;
use goodfetch_test_client::{Attached, hello};
use goodfetch_test_support::{TestEnv, assert_cli_snapshot, browser_cache_in_home, cli};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The settings of a home whose Runs launch Chrome for Testing and may
/// download nothing.
const OFFLINE_FULL_BUILD: &str = "browser.headless_shell = false\negress.offline = true\n";

/// Each finding's part and status.
fn statuses(report: &DoctorReport) -> Vec<(FindingId, FindingStatus)> {
    report
        .findings
        .iter()
        .map(|finding| (finding.id, finding.status))
        .collect()
}

/// A home whose Browser cache lacks the build a Run launches, with
/// `egress.offline` on, has a failing `browsers` finding with its fix;
/// `doctor` still exits `0`, under `--json` and for people.
///
/// Covers: story:11-212, command:doctor
#[test]
fn doctor_exits_0_and_prints_a_fix_for_each_failing_finding() {
    let env = TestEnv::new();
    env.user_configuration(OFFLINE_FULL_BUILD);

    let checked = cli(&env, COLLIE)
        .args(["doctor", "--json"])
        .json("DoctorReport")
        .unwrap();
    let human = cli(&env, COLLIE).args(["doctor"]).run().unwrap();

    let report: DoctorReport = serde_json::from_value(checked.documents[0].clone()).unwrap();
    let unfixed: Vec<FindingId> = report
        .findings
        .iter()
        .filter(|finding| finding.status != FindingStatus::Ok && finding.fix.is_none())
        .map(|finding| finding.id)
        .collect();
    assert_eq!(
        (checked.output.exit, human.exit, statuses(&report), unfixed),
        (
            Some(0),
            Some(0),
            vec![
                (FindingId::Home, FindingStatus::Ok),
                (FindingId::Executables, FindingStatus::Ok),
                (FindingId::Daemon, FindingStatus::Ok),
                (FindingId::VersionSkew, FindingStatus::Ok),
                (FindingId::Browsers, FindingStatus::Failing),
            ],
            vec![],
        )
    );
    assert_cli_snapshot!("doctor_json", checked.output);
    assert_cli_snapshot!("doctor_human", human);
}

/// The build a Run launches, laid out in the home's Browser cache, makes
/// the `browsers` finding `ok`, with no fix.
///
/// Covers: story:11-212, command:doctor
#[test]
fn doctor_finds_the_build_a_run_launches_in_the_browser_cache() {
    let env = TestEnv::new();
    env.user_configuration(OFFLINE_FULL_BUILD);
    browser_cache_in_home(&env).unwrap();

    let checked = cli(&env, COLLIE)
        .args(["doctor", "--json"])
        .json("DoctorReport")
        .unwrap();

    let report: DoctorReport = serde_json::from_value(checked.documents[0].clone()).unwrap();
    let browsers = report
        .findings
        .iter()
        .find(|finding| finding.id == FindingId::Browsers)
        .map(|finding| (finding.status, finding.fix.clone()));
    assert_eq!(browsers, Some((FindingStatus::Ok, None)));
}

/// What `doctor --json`, with `arguments` after it, printed and the
/// bug-report file it wrote.
fn bug_report(env: &TestEnv, arguments: &[&str]) -> (Option<i32>, BugReportFile, Value) {
    let checked = cli(env, COLLIE)
        .args(["doctor", "--json"])
        .args(arguments)
        .json("DoctorReport")
        .unwrap();
    let report: DoctorReport = serde_json::from_value(checked.documents[0].clone()).unwrap();
    let file = std::fs::read(env.home().join(BUG_REPORT)).unwrap();
    (
        checked.output.exit,
        report.bug_report,
        serde_json::from_slice(&file).unwrap(),
    )
}

/// The value the bug-report file `report` holds for the setting `key`.
fn setting<'r>(report: &'r Value, key: &str) -> Option<&'r Value> {
    report["settings"]
        .as_array()?
        .iter()
        .find(|setting| setting["key"] == key)
        .map(|setting| &setting["value"])
}

/// `doctor` writes the bug-report file in Goodfetch home and names it: the
/// versions, the settings a layer set, a credential as its Secret
/// reference, each URL reduced to its scheme and host unless
/// `--include-urls` keeps it whole, and the Error code of a call that
/// failed before. It sends nothing: the OTLP collector the settings name
/// records no connection.
///
/// Covers: story:4-84, command:doctor, setting:otlp.traces_endpoint
#[test]
fn doctor_writes_a_bug_report_file_and_sends_nothing() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "sample.crawl", "params": {} }));
    let refused = engine.next().unwrap();
    let _ = engine.finish();
    let (endpoint, _, connections) =
        crate::log::collector_of::<ExportTraceServiceRequest>("/v1/traces");
    env.user_configuration(&format!(
        "otlp.traces_endpoint = \"{endpoint}\"\nproxy.server = \"http://proxy.example.test:3128/\"\nproxy.password = \"env:GOODFETCH_TEST_PROXY_PASSWORD\"\n"
    ));

    let (exit, written, reduced) = bug_report(&env, &[]);
    let (whole_exit, _, whole) = bug_report(&env, &["--include-urls"]);

    let failed = reduced["recent_errors"].as_array().is_some_and(|errors| {
        errors
            .iter()
            .any(|error| error["code"] == "not_in_product" && error["method"] == "sample.crawl")
    });
    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            exit,
            whole_exit,
            written,
            (&reduced["product"], &reduced["product_version"]),
            setting(&reduced, "otlp.traces_endpoint"),
            setting(&reduced, "proxy.server"),
            setting(&reduced, "proxy.password"),
            failed,
            setting(&whole, "otlp.traces_endpoint"),
            setting(&whole, "proxy.server"),
            connections.try_iter().count(),
        ),
        (
            &json!("not_in_product"),
            Some(0),
            Some(0),
            BugReportFile::Written {
                path: env.home().join(BUG_REPORT).display().to_string(),
            },
            (&json!("collie"), &json!("0.0.0")),
            Some(&json!("http://127.0.0.1")),
            Some(&json!("http://proxy.example.test")),
            Some(&json!("env:GOODFETCH_TEST_PROXY_PASSWORD")),
            true,
            Some(&json!(endpoint)),
            Some(&json!("http://proxy.example.test:3128/")),
            0,
        )
    );
}
