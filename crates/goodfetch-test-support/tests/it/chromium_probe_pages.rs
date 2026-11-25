//! The Goodfetch Chromium's test helpers driving the Proving ground's probe
//! pages with a stock Chromium (ADR 0074): the pinned Chrome for Testing
//! release, headless, every `.test` host mapped to the Proving ground.
//! A stock browser patches nothing, so every surface agrees in every
//! context and header, and the helpers read what its pages saw: the
//! consistency probes' reports, the codec samples it fetched, the fonts it
//! measured and, from its NetLog, the sockets it connected. Linux x86-64,
//! where `browser_cache` lays out the build.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_test_support::{
    CODECS_HOST, CONSISTENCY_HOST, CodecSample, DebuggedBrowser, FONTS_HOST, FontAvailability,
    IdentityReport, NetLog, PageValues, ProvingGround, ReceivedRequests, SocketTransport, TestEnv,
    browser_cache, chrome_for_testing_debugged, chrome_for_testing_dumping_dom, identity_report,
};
use serde_json::Value;

/// Every surface the consistency probes read.
const SURFACES: [&str; 28] = [
    "user_agent",
    "app_version",
    "platform",
    "language",
    "languages",
    "hardware_concurrency",
    "device_memory",
    "webdriver",
    "time_zone",
    "locale",
    "timezone_offset",
    "ua_brands",
    "ua_mobile",
    "ua_platform",
    "ua_platform_version",
    "ua_architecture",
    "ua_bitness",
    "ua_model",
    "ua_wow64",
    "ua_full_version_list",
    "webgl_vendor",
    "webgl_renderer",
    "screen_width",
    "screen_height",
    "screen_avail_width",
    "screen_avail_height",
    "screen_color_depth",
    "device_pixel_ratio",
];

/// The codecs a stock Chrome for Testing build decodes, as an Identity
/// profile would claim them.
const CLAIMED_CODECS: [&str; 7] = ["h264", "vp8", "vp9", "av1", "aac", "opus", "flac"];

/// The font families a fixture Identity profile lists.
const LISTED_FAMILIES: [&str; 2] = ["DejaVu Sans", "Goodfetch Test Sans"];

/// How long a page may take to reach the state a test waits for: the codec
/// page gives each video five seconds to show a frame.
const PAGE_DEADLINE: Duration = Duration::from_secs(90);

/// How often the Proving ground's observations are read while waiting.
const POLL: Duration = Duration::from_millis(200);

/// How long the browser may take to write its NetLog and exit once
/// interrupted.
const EXIT_DEADLINE: Duration = Duration::from_secs(30);

/// The Proving ground's observations once `reached` finds the state named
/// `state` in them, read again until [`PAGE_DEADLINE`].
fn once<T>(
    proving_ground: &ProvingGround,
    state: &str,
    reached: impl Fn(&Value) -> Option<T>,
) -> T {
    let deadline = Instant::now() + PAGE_DEADLINE;
    loop {
        let observations = proving_ground.observations().unwrap();
        if let Some(found) = reached(&observations) {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{state} within {PAGE_DEADLINE:?}"
        );
        std::thread::sleep(POLL);
    }
}

/// Opens `url` in a new tab of `browser`.
fn open(browser: &DebuggedBrowser, url: &str) {
    ureq::put(format!("http://127.0.0.1:{}/json/new?{url}", browser.port))
        .send_empty()
        .unwrap();
}

/// Each surface's report, once every context has reported every surface.
fn every_surface_reported(observations: &Value) -> Option<Vec<IdentityReport>> {
    let reports: Vec<IdentityReport> = PageValues::read(observations)
        .unwrap()
        .iter()
        .map(identity_report)
        .collect();
    let complete: BTreeSet<&str> = reports
        .iter()
        .filter(|report| report.unreported.is_empty())
        .map(|report| report.surface.as_str())
        .collect();
    SURFACES
        .iter()
        .all(|surface| complete.contains(surface))
        .then_some(reports)
}

/// Whether every path of `paths` was requested of `host`.
fn requested(observations: &Value, host: &str, paths: &[String]) -> bool {
    let requests = ReceivedRequests::of(observations);
    paths.iter().all(|path| {
        requests
            .of_the_case()
            .any(|request| request.host == host && request.target == *path)
    })
}

/// The DOM a fresh stock browser dumps of `url` on [`FONTS_HOST`], once its
/// page is measured.
fn fonts_dom(env: &TestEnv, cache: &Path, proving_ground: &ProvingGround, url: &str) -> String {
    let browser = chrome_for_testing_dumping_dom(
        env,
        cache,
        (FONTS_HOST, proving_ground.address().ip()),
        &["--virtual-time-budget=10000"],
        url,
    )
    .unwrap();
    let mut dom = String::new();
    let stdout = browser.child().stdout.take();
    std::io::Read::read_to_string(&mut stdout.unwrap(), &mut dom).unwrap();
    let status = browser.close_stdin_and_wait().unwrap();
    assert!(status.success(), "the browser exited with {status}");
    dom
}

/// The `#results` JSON of a dumped DOM.
fn results(dom: &str) -> Value {
    let start = dom.find("<pre id=\"results\">").unwrap() + "<pre id=\"results\">".len();
    let end = start + dom[start..].find("</pre>").unwrap();
    let text = dom[start..end]
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn a_stock_chromium_agrees_with_itself_in_every_context_and_its_sockets_reach_the_proving_ground() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let proving_ground = ProvingGround::start(&env, 404).unwrap();
    let net_log = env.folder().join("net-log.json");
    let browser = chrome_for_testing_debugged(
        &env,
        &cache,
        &env.folder().join("profile"),
        [
            "--ignore-certificate-errors".to_owned(),
            format!(
                "--host-resolver-rules=MAP *.test {}",
                proving_ground.address().ip()
            ),
            format!("--log-net-log={}", net_log.display()),
        ],
    )
    .unwrap();
    let port = proving_ground.address().port();

    open(&browser, &format!("https://{CONSISTENCY_HOST}:{port}/"));
    let reports = once(
        &proving_ground,
        "every context's report of every surface",
        every_surface_reported,
    );
    let samples = proving_ground.codec_samples(&CLAIMED_CODECS).unwrap();
    let paths: Vec<String> = samples.iter().map(CodecSample::path).collect();
    open(&browser, &format!("https://{CODECS_HOST}:{port}/"));
    once(
        &proving_ground,
        "every claimed codec's sample fetched",
        |observations| requested(observations, CODECS_HOST, &paths).then_some(()),
    );
    browser.process.interrupt_and_wait(EXIT_DEADLINE).unwrap();
    let log = NetLog::read(&net_log).unwrap();

    let inconsistent: Vec<String> = reports
        .iter()
        .filter(|report| !report.is_consistent())
        .map(IdentityReport::to_string)
        .collect();
    assert!(inconsistent.is_empty(), "{inconsistent:#?}");
    let surfaces: BTreeSet<&str> = reports
        .iter()
        .map(|report| report.surface.as_str())
        .collect();
    assert_eq!(surfaces, BTreeSet::from(SURFACES));
    assert!(
        log.sockets().iter().any(|socket| {
            socket.transport == SocketTransport::Tcp && socket.address == proving_ground.address()
        }),
        "{:?}",
        log.sockets()
    );
}

/// The font page a stock browser measures holds the fonts `font_samples`
/// lists, each with the result it expects.
#[test]
fn a_stock_chromium_measures_the_fonts_font_samples_lists() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let proving_ground = ProvingGround::start(&env, 404).unwrap();
    let samples = proving_ground.font_samples(&LISTED_FAMILIES).unwrap();
    let url = format!(
        "http://{FONTS_HOST}:{}{}",
        proving_ground.address().port(),
        samples.path
    );

    let measured = results(&fonts_dom(&env, &cache, &proving_ground, &url));

    let fonts = measured["fonts"].as_object().unwrap();
    let listed: Vec<(&str, bool, &str)> = samples
        .fonts
        .iter()
        .map(|font| {
            let expected = match font.expected {
                FontAvailability::Available => "available",
                FontAvailability::Unavailable => "unavailable",
            };
            (font.family.as_str(), font.listed, expected)
        })
        .collect();
    let shown: Vec<(&str, bool, &str)> = samples
        .fonts
        .iter()
        .map(|font| {
            let shown = &fonts[&font.family];
            (
                font.family.as_str(),
                shown["listed"].as_bool().unwrap(),
                shown["expected"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(fonts.len(), samples.fonts.len());
    assert_eq!(shown, listed);
}
