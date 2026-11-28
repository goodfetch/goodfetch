//! J-049, "Crawl offline with browsers installed beforehand, a missing
//! artifact refused with `artifact_unavailable_offline`", on the `cli` surface
//! (ADRs 0043 and 0063; spec #565, ticket #687): a person installs Foxhound's
//! pinned browser build beforehand through `browser install`, runs a crawl
//! with `egress.offline` turned on using that pre-installed browser, and then
//! runs a crawl with `egress.offline` turned on against an empty Browser
//! cache and sees `artifact_unavailable_offline` naming `browser install`,
//! with zero download connections during either offline crawl.
#![expect(
    clippy::disallowed_methods,
    reason = "the person's steps create the empty cache directory, write settings files, and read the result document outside any async worker"
)]

use std::path::Path;

use goodfetch_protocol::journey::Journey;
use goodfetch_protocol::result_document::{
    LoadedDocument, ResultDocument, read, read_journey_document,
};
use goodfetch_test_support::{
    BROWSER_BUILDS_HOST, BROWSERS_PATH_VARIABLE, JourneyRun, ReceivedRequests, cli as command_line,
};
use serde_json::{Value, json};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// Stops the Daemon when the test ends, whether it passed or not.
struct StopsTheDaemon<F: FnMut()>(F);

impl<F: FnMut()> Drop for StopsTheDaemon<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

/// How many download requests the Proving ground's mirror has received.
fn mirror_requests(observations: &Value) -> usize {
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| request.host == BROWSER_BUILDS_HOST)
        .count()
}

/// Covers: story:4-82
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant reads as its file: each step, then each check, in order"
)]
fn cli() {
    let mut run = JourneyRun::start_downloading_browser(
        "J-049",
        "cli",
        Path::new(FOXHOUND),
        ResultDocument::journey,
    )
    .expect("J-049's cli variant starts");
    let proving_ground = run.proving_ground().expect("J-049 has a Proving ground");
    let page_url = proving_ground.url("scope.test", "/");
    let mut stop = run.command();
    stop.args(["daemon", "stop"]);
    let _stops = StopsTheDaemon(move || {
        let _ = stop.output();
    });

    let mut downloads_after_install = 0usize;
    let mut downloads_after_offline_crawl = 0usize;
    let mut downloads_after_missing_crawl = 0usize;
    let mut offline_records: Vec<Value> = Vec::new();
    let mut missing_outcome = Value::Null;

    run.step(
        "Install the pinned browser build into the Browser cache with `browser install`",
        |run| {
            let output = run
                .command()
                .args(["browser", "install", "--json"])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            downloads_after_install = mirror_requests(
                &run.proving_ground()
                    .expect("J-049 has a Proving ground")
                    .observations()
                    .unwrap(),
            );
            assert!(
                downloads_after_install >= 1,
                "expected browser install to download from the mirror"
            );
        },
    );
    run.step(
        "Crawl a site with `egress.offline` turned on using the installed browser build",
        |run| {
            let settings_path = run.env().folder().join("offline.json");
            std::fs::write(
                &settings_path,
                json!({
                    "fetch": { "tier": "browser" },
                    "egress": { "offline": true },
                    "crawl": { "retry": { "max": 0 } },
                    "politeness": { "domain": { "delay": 0 } },
                    "site_preferences": { "respect_robots_txt": false },
                })
                .to_string(),
            )
            .unwrap();
            let started = command_line(run.env(), run.engine())
                .args(["crawl", "start", &page_url, "--run-id", "offline-ok"])
                .args(["--format", "markdown", "--links", "none"])
                .args(["--settings", settings_path.to_str().unwrap(), "--json"])
                .json("RunId")
                .unwrap();
            assert_eq!(started.documents[0], "offline-ok", "{}", started.output);
            let ended = command_line(run.env(), run.engine())
                .args(["run", "outcomes", "offline-ok", "--follow", "--json"])
                .json_lines("CrawlRequestOutcome")
                .unwrap();
            assert_eq!(ended.documents.len(), 1, "{}", ended.output);
            assert_eq!(ended.documents[0]["kind"], "succeeded", "{}", ended.output);
            let read_records = command_line(run.env(), run.engine())
                .args([
                    "run",
                    "records",
                    "offline-ok",
                    "--dataset",
                    "pages",
                    "--json",
                ])
                .json_array("RecordRead")
                .unwrap();
            offline_records = read_records.documents[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["record"].clone())
                .collect();
            downloads_after_offline_crawl = mirror_requests(
                &run.proving_ground()
                    .expect("J-049 has a Proving ground")
                    .observations()
                    .unwrap(),
            );
        },
    );
    run.step(
        "Run a crawl with `egress.offline` turned on against an empty Browser cache that needs a missing artifact",
        |run| {
            // Stop the Daemon from step 2 so the next command starts an engine
            // pointed at the empty Browser cache.
            let stopped = run.command().args(["daemon", "stop"]).output().unwrap();
            assert!(stopped.status.success(), "{stopped:?}");

            let empty_cache = run.env().folder().join("empty-browser-cache");
            std::fs::create_dir_all(&empty_cache).unwrap();
            let settings_path = run.env().folder().join("offline.json");
            let started = command_line(run.env(), run.engine())
                .env(BROWSERS_PATH_VARIABLE, &empty_cache)
                .args(["crawl", "start", &page_url, "--run-id", "offline-missing"])
                .args(["--format", "markdown", "--links", "none"])
                .args(["--settings", settings_path.to_str().unwrap(), "--json"])
                .json("RunId")
                .unwrap();
            assert_eq!(
                started.documents[0], "offline-missing",
                "{}",
                started.output
            );
            let ended = command_line(run.env(), run.engine())
                .env(BROWSERS_PATH_VARIABLE, &empty_cache)
                .args(["run", "outcomes", "offline-missing", "--follow", "--json"])
                .json_lines("CrawlRequestOutcome")
                .unwrap();
            assert_eq!(ended.documents.len(), 1, "{}", ended.output);
            missing_outcome = ended.documents[0].clone();
            downloads_after_missing_crawl = mirror_requests(
                &run.proving_ground()
                    .expect("J-049 has a Proving ground")
                    .observations()
                    .unwrap(),
            );
        },
    );

    run.check(
        "The offline crawl with the pre-installed browser finishes and returns the site's Records with no download connection",
        |_| {
            assert_eq!(offline_records.len(), 1, "{offline_records:?}");
            assert_eq!(downloads_after_offline_crawl, downloads_after_install);
        },
    );
    run.check(
        "The crawl missing its browser artifact is refused with `artifact_unavailable_offline` naming `browser install`, with no download connection",
        |_| {
            let error = &missing_outcome["error"];
            let fix = error["details"]["fix"].as_str().unwrap_or_default();
            let message = error["message"].as_str().unwrap_or_default();
            assert_eq!(
                (
                    missing_outcome["kind"].clone(),
                    error["code"].clone(),
                    fix.contains("browser install") || message.contains("browser install"),
                    downloads_after_missing_crawl,
                ),
                (
                    json!("failed"),
                    json!("artifact_unavailable_offline"),
                    true,
                    downloads_after_offline_crawl,
                ),
                "{missing_outcome}"
            );
        },
    );

    let finished = run.finish().expect("J-049's document is written");
    finished.assert_proved();
    let journey: Journey = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../journeys/J-049.toml"
    )))
    .unwrap();
    let bytes = std::fs::read(finished.path()).unwrap();
    let LoadedDocument::Journey(loaded) = read(&bytes).unwrap() else {
        panic!("J-049's document is a journey document");
    };
    read_journey_document(loaded, &journey).expect("the reader accepts J-049's document");
}
