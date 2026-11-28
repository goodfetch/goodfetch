//! J-046, "Develop against the Response cache, then test offline from a
//! recording", on the `sdk-rust` surface (ADRs 0046, 0063, 0069 and 0105;
//! ticket #808): a Rust program drives Collie's shipped executable as a Daemon,
//! runs against a Proving ground product page twice under `cache.mode: replay`
//! (the second run sending zero requests to the Proving ground), records a
//! Response recording of the program, and runs it again offline from the
//! recording with `recording.mode: replay` (sending zero requests and listing
//! no miss in its Run report). TypeScript's `tests/journeys/J-046.test.ts` and
//! Python's `tests/journeys/test_j046.py` are its twins.

use std::collections::BTreeMap;

use goodfetch_collie::settings::{Settings, cache, network};
use goodfetch_collie::{
    Collie, ConnectOptions, EngineChoice, EngineKind, FieldLocator, FieldTransform, PageField,
    Recording, RecordingMode, RunReport, ValueField,
};
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{
    JourneyRun, ReceivedRequests, Tool, browser_cache_in_home, real_time_runtime, tools,
};
use serde_json::{Map, Value, json};

/// The Proving ground's task whose product page the program reads.
const TASK: &str = "catalog/2";

/// Stops the Daemon `daemon start` launched when the test ends, whether it
/// passed or not.
struct StopsTheDaemon<F: FnMut()>(F);

impl<F: FnMut()> Drop for StopsTheDaemon<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// The product's ID, name and price fields, matching task `catalog/2`.
fn product_fields() -> BTreeMap<String, PageField> {
    BTreeMap::from([
        (
            "id".to_owned(),
            PageField::Value(ValueField {
                locator: FieldLocator::Css {
                    selector: "span.id".to_owned(),
                },
                fallbacks: Vec::new(),
                attribute: None,
                matched: None,
                transform: FieldTransform::Text,
            }),
        ),
        (
            "name".to_owned(),
            PageField::Value(ValueField {
                locator: FieldLocator::Css {
                    selector: "h1.name".to_owned(),
                },
                fallbacks: Vec::new(),
                attribute: None,
                matched: None,
                transform: FieldTransform::Text,
            }),
        ),
        (
            "price".to_owned(),
            PageField::Value(ValueField {
                locator: FieldLocator::Css {
                    selector: "span.price".to_owned(),
                },
                fallbacks: Vec::new(),
                attribute: Some("data-price".to_owned()),
                matched: None,
                transform: FieldTransform::Number {
                    locale: "en".to_owned(),
                },
            }),
        ),
    ])
}

/// Runs the program once with `settings` and `recording`, reading `url`'s
/// product fields and returning the read values and finished Run report.
async fn run_program(
    collie: &Collie,
    url: &str,
    settings: Option<Settings>,
    recording: Option<Recording>,
) -> (Map<String, Value>, RunReport) {
    let mut start = collie.start();
    if let Some(settings) = settings {
        start = start.settings(settings);
    }
    if let Some(recording) = recording {
        start = start.recording(recording);
    }
    let started = start.await.expect("program Run starts");
    let page = started.new_page().await.expect("program page opens");
    page.goto(url).await.expect("program navigates to product");
    let read = page
        .read(product_fields())
        .await
        .expect("program reads product fields");
    let report = started.finish().await.expect("program Run finishes");
    (read.values.clone(), (*report).clone())
}

/// Total requests the Proving ground has observed so far.
fn total_requests<C>(run: &JourneyRun<C>) -> usize {
    let observations = run
        .proving_ground()
        .expect("J-046 has a Proving ground")
        .observations()
        .expect("Proving ground observations are readable");
    ReceivedRequests::of(&observations).every().count()
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn sdk_rust() {
    let manifest = tools().expect("test tools manifest is set");
    let engine = manifest
        .path(Tool::Collie)
        .expect("collie executable is in test tools manifest");
    let mut run = JourneyRun::start("J-046", "sdk-rust", engine, ResultDocument::journey)
        .expect("J-046's sdk-rust variant starts");
    run.surface_package("goodfetch-collie", env!("CARGO_PKG_VERSION"));

    let proving_ground = run.proving_ground().expect("J-046 has a Proving ground");
    browser_cache_in_home(run.env()).expect("Playwright's Chrome for Testing is installed");
    run.env()
        .user_configuration(&proving_ground.description().reaching());
    let product_url = proving_ground
        .start_url(TASK)
        .expect("catalog/2 start URL is served");
    let warc_path = run
        .env()
        .folder()
        .join("product.warc")
        .to_string_lossy()
        .into_owned();

    // Start Collie's shipped executable as a Daemon so `sdk-rust` drives it
    // over its public Daemon boundary rather than linking an in-process engine
    // (ADR 0069; TESTING.md, Journeys).
    let started_daemon = run
        .command()
        .args(["daemon", "start", "--json"])
        .output()
        .expect("daemon start runs");
    assert!(
        started_daemon.status.success(),
        "daemon start failed: {started_daemon:?}"
    );
    let mut stop_daemon = run.command();
    stop_daemon.args(["daemon", "stop", "--json"]);
    let _stop = StopsTheDaemon(move || {
        let _ = stop_daemon.output();
    });

    let env_os = run.env().env_os();
    let runtime = real_time_runtime();
    let collie = runtime.block_on(async {
        let collie = Collie::connect(
            ConnectOptions::default()
                .engine(EngineChoice::Daemon)
                .environment(env_os),
        )
        .await
        .expect("Rust SDK connects to the started Daemon");
        assert_eq!(collie.engine().kind, EngineKind::Daemon);
        collie
    });

    let cache_replay = || Settings::new().set(cache::MODE, cache::Mode::Replay);
    let mut first_values: Option<Map<String, Value>> = None;
    let mut second_values: Option<Map<String, Value>> = None;
    let mut offline_values: Option<Map<String, Value>> = None;
    let mut offline_report: Option<RunReport> = None;
    let mut requests_after_first = 0usize;
    let mut requests_after_second = 0usize;
    let mut requests_after_record = 0usize;
    let mut requests_after_offline = 0usize;

    run.step(
        "Run the program against a product page of the site under cache replay",
        |run| {
            let (values, _) = runtime.block_on(run_program(
                &collie,
                &product_url,
                Some(cache_replay()),
                None,
            ));
            first_values = Some(values);
            requests_after_first = total_requests(run);
        },
    );
    run.step(
        "Run the same program a second time under cache replay on the same Response cache",
        |run| {
            let (values, _) = runtime.block_on(run_program(
                &collie,
                &product_url,
                Some(cache_replay()),
                None,
            ));
            second_values = Some(values);
            requests_after_second = total_requests(run);
        },
    );
    run.step(
        "Record a Response recording of the program against the site",
        |run| {
            let recording = Recording {
                mode: RecordingMode::Record,
                path: warc_path.clone(),
            };
            let _ = runtime.block_on(run_program(&collie, &product_url, None, Some(recording)));
            requests_after_record = total_requests(run);
        },
    );
    run.step(
        "Run the same program offline from the Response recording",
        |run| {
            let offline_settings = Settings::new().set(network::RESOLVE, BTreeMap::new());
            let recording = Recording {
                mode: RecordingMode::Replay,
                path: warc_path.clone(),
            };
            let (values, report) = runtime.block_on(run_program(
                &collie,
                &product_url,
                Some(offline_settings),
                Some(recording),
            ));
            offline_values = Some(values);
            offline_report = Some(report);
            requests_after_offline = total_requests(run);
        },
    );

    runtime
        .block_on(collie.close())
        .expect("Rust SDK closes cleanly");

    let first_records = first_values.map_or_else(Vec::new, |v| vec![Value::Object(v)]);
    run.expect_task(
        "The oracle verifies the product the first run reads",
        &json!({ "status": "success", "records": first_records }),
    );
    let second_records = second_values.map_or_else(Vec::new, |v| vec![Value::Object(v)]);
    run.expect_task(
        "The oracle verifies the product the second run reads from the Response cache",
        &json!({ "status": "success", "records": second_records }),
    );
    run.check(
        "The second run under cache replay sends the Proving ground no request",
        |_| {
            assert!(
                requests_after_first > 0,
                "first run must fetch from the Proving ground"
            );
            assert_eq!(
                requests_after_second, requests_after_first,
                "second run under cache.mode: replay sent requests to the Proving ground"
            );
        },
    );
    let offline_records = offline_values.map_or_else(Vec::new, |v| vec![Value::Object(v)]);
    run.expect_task(
        "The oracle verifies the product the offline run reads from the Response recording",
        &json!({ "status": "success", "records": offline_records }),
    );
    run.check(
        "The offline run from the Response recording sends the Proving ground no request and lists no miss in its Run report",
        |_| {
            assert!(
                requests_after_record > requests_after_second,
                "record run must fetch from the Proving ground"
            );
            assert_eq!(
                requests_after_offline, requests_after_record,
                "offline replay run sent requests to the Proving ground"
            );
            let recording = offline_report
                .as_ref()
                .and_then(|report| report.recording.as_ref())
                .expect("offline Run report carries `recording`");
            assert_eq!(recording.mode, "replay");
            assert!(
                recording.responses_used > 0,
                "offline replay must answer at least one recorded response"
            );
            assert!(
                recording.misses.is_empty(),
                "offline replay listed misses: {:?}",
                recording.misses
            );
        },
    );

    run.finish()
        .expect("J-046's document is written")
        .assert_proved();
}
