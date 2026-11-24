//! The consistency probe pages (#328, ADR 0074) on `consistency.test` and
//! `consistency-frame.test`: each context, the main frame, a cross-origin
//! iframe and dedicated, shared and service workers, reports each surface's
//! value to the site, whose observations keep it beside the request that
//! carried it and that request's headers. A plain HTTP client reads the
//! pages and posts a report; a stock Chromium, never Goodfetch, runs them
//! all, and a stock browser's contexts agree with each other and with the
//! headers they sent.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 328;

const HOST: &str = "consistency.test";
const FRAME_HOST: &str = "consistency-frame.test";

/// Each context the probes run in, with the page it reports as.
const CONTEXTS: [(&str, &str); 5] = [
    ("main", "/"),
    ("frame", "/frame"),
    ("dedicated_worker", "/dedicated-worker.js"),
    ("shared_worker", "/shared-worker.js"),
    ("service_worker", "/service-worker.js"),
];

/// The surfaces every context of a stock Chromium can read.
const SHARED_SURFACES: [&str; 8] = [
    "user_agent",
    "platform",
    "language",
    "languages",
    "hardware_concurrency",
    "time_zone",
    "locale",
    "ua_brands",
];

/// The reports the observations hold, each `(page, check, observed)`.
fn reports(observations: &Value) -> Vec<(String, String, String)> {
    observations["reports"]
        .as_array()
        .expect("the observations list reports")
        .iter()
        .map(|report| {
            let field = |name: &str| report[name].as_str().expect("a report's field").to_owned();
            (field("page"), field("check"), field("observed"))
        })
        .collect()
}

/// The `User-Agent` headers of the reports the page `page` posted to `host`.
fn report_user_agents(observations: &Value, host: &str, page: &str) -> Vec<String> {
    let target = format!("/reports?page={page}");
    observations["requests"]
        .as_array()
        .expect("the observations list requests")
        .iter()
        .filter(|request| request["host"] == host && request["target"] == target.as_str())
        .filter_map(|request| {
            request["headers"]
                .as_array()
                .expect("a request lists its headers")
                .iter()
                .find(|header| {
                    header[0]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("user-agent"))
                })
                .map(|header| header[1].as_str().expect("a header's value").to_owned())
        })
        .collect()
}

/// Covers: story:2-91
#[test]
fn a_plain_client_reads_every_probe_and_its_report_is_recorded() {
    let proving_ground = ProvingGround::start(SEED);

    let main = proving_ground.get(&proving_ground.url(HOST, "/"));
    let frame = proving_ground.get(&proving_ground.url(FRAME_HOST, "/frame"));
    let scripts: Vec<_> = [
        "/probe.js",
        "/dedicated-worker.js",
        "/shared-worker.js",
        "/service-worker.js",
    ]
    .into_iter()
    .map(|path| {
        let script = proving_ground.get_raw(&proving_ground.url(HOST, path));
        (path, script.status, script.content_type)
    })
    .collect();
    let report = json!({ "page": "/", "check": "user_agent", "observed": "\"Probe/1\"" });
    let posted = proving_ground.post_json(&proving_ground.url(HOST, "/reports?page=/"), &report);
    let malformed = proving_ground.post_json(
        &proving_ground.url(HOST, "/reports?page=/"),
        &json!({ "page": "/", "check": "", "observed": "x" }),
    );
    let unknown = proving_ground.get(&proving_ground.url(HOST, "/elsewhere"));

    assert_eq!(main.status, 200);
    for part in [
        "src=\"/probe.js\"",
        "consistency-frame.test",
        "new Worker(\"/dedicated-worker.js\")",
        "new SharedWorker(\"/shared-worker.js\")",
        "serviceWorker.register(\"/service-worker.js\")",
    ] {
        assert!(
            main.body.contains(part),
            "the main page has {part}: {}",
            main.body
        );
    }
    assert_eq!(frame.status, 200);
    assert!(frame.body.contains("src=\"/probe.js\""), "{}", frame.body);
    for (path, status, content_type) in scripts {
        assert_eq!(
            (status, content_type.as_deref()),
            (200, Some("application/javascript")),
            "{path}"
        );
    }
    assert_eq!(
        (posted.status, malformed.status, unknown.status),
        (204, 400, 404)
    );
    assert_eq!(
        reports(&proving_ground.observations()),
        [(
            "/".to_owned(),
            "user_agent".to_owned(),
            "\"Probe/1\"".to_owned()
        )]
    );
}

/// Covers: story:2-91
#[test]
fn a_stock_chromium_reports_the_same_values_from_every_context_and_its_headers() {
    let proving_ground = ProvingGround::start(SEED);
    // A service worker and the user-agent client hints need a secure
    // context, so the pages are read over TLS. The stock browser does not
    // trust the Proving ground's authority, and registers no service worker
    // past a certificate error unless the browser ignores them all; it
    // reaches only the loopback address.
    let shown = browser::run(
        "consistency",
        "probe",
        &format!("https://{HOST}:{}/", proving_ground.port()),
        json!({ "args": ["--ignore-certificate-errors"] }),
    );
    let observations = proving_ground.observations();

    let values = &shown["contexts"];
    for (context, _) in CONTEXTS {
        for surface in SHARED_SURFACES {
            assert!(
                !values[context][surface].is_null(),
                "{context} reads {surface}: {shown}"
            );
        }
    }
    for (context, _) in CONTEXTS {
        let surfaces = values[context]
            .as_object()
            .expect("each context shows its surfaces");
        for (surface, value) in surfaces {
            if value.is_null() {
                continue;
            }
            assert_eq!(
                value, &values["main"][surface],
                "{context}'s {surface} is the main frame's"
            );
        }
    }
    let mut recorded: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for (page, check, observed) in reports(&observations) {
        let value = serde_json::from_str(&observed).expect("each value is reported as JSON");
        recorded.entry(page).or_default().insert(check, value);
    }
    for (context, page) in CONTEXTS {
        let shown_values: BTreeMap<String, Value> = values[context]
            .as_object()
            .expect("each context shows its surfaces")
            .iter()
            .map(|(surface, value)| (surface.clone(), value.clone()))
            .collect();
        assert_eq!(
            recorded.get(page),
            Some(&shown_values),
            "{context} reported each surface it read"
        );
        let host = if context == "frame" { FRAME_HOST } else { HOST };
        let user_agent = values[context]["user_agent"]
            .as_str()
            .expect("a user agent");
        let headers = report_user_agents(&observations, host, page);
        assert!(!headers.is_empty(), "{context}'s reports were observed");
        assert!(
            headers.iter().all(|header| header == user_agent),
            "{context}'s requests send the user agent it reads: {headers:?}"
        );
    }
}
