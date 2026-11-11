//! The rung site on `rungs.localhost`: pages of three kinds, each needing
//! one rung of `auto`'s ladder. A static page is the same to every client;
//! a browser-identity page answers its content only to the TLS fingerprint
//! of the Chrome for Testing build Collie pins, and refuses any other
//! client; a script-rendered page holds its Record only once its script
//! ran. Its manifest lists every page with its kind and its expected
//! Records. It belongs to no task and declares no Feature.
//!
//! The pinned browser here is the build test support lays out in a
//! Browser cache, started once per page with `--dump-dom`, never an
//! engine crate.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::Path;
use std::process::Stdio;

use goodfetch_test_support::{Process, TestEnv, browser_cache, chrome_for_testing};
use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::clients;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 2410;

const HOST: &str = "rungs.localhost";

/// The pages of each kind.
const PAGES_PER_KIND: u64 = 6;

/// The switch Collie starts its browsers with that changes their TLS
/// fingerprint: without the field-trial configuration built into the
/// binary, Chrome sends one extension fewer.
const AS_COLLIE_STARTS_IT: &[&str] = &["--disable-field-trial-config"];

/// The browsers started at once by a crawl.
const BROWSERS_AT_ONCE: usize = 6;

/// The Records in a page's markup, each with its `id` and `name`.
fn records(html: &str) -> Vec<Value> {
    Regex::new(
        r#"<article class="item"[^>]*>\s*<h1 class="name">([^<]*)</h1>\s*<p class="id">([^<]*)</p>"#,
    )
    .expect("a valid pattern")
    .captures_iter(html)
    .map(|item| json!({ "id": item[2], "name": item[1] }))
    .collect()
}

/// The paths a page's markup links.
fn links(html: &str) -> BTreeSet<String> {
    Regex::new(r#"<a href="(/[^"]*)""#)
        .expect("a valid pattern")
        .captures_iter(html)
        .map(|link| link[1].to_owned())
        .collect()
}

/// What reqwest gets for `path` over TLS, on the rustls configuration the
/// HTTP fetch tier gives it: the status and the body.
fn reqwest_gets(proving_ground: &ProvingGround, path: &str) -> (u16, String) {
    let tls = clients::trusting(&proving_ground.certificate_authority(), &["h2", "http/1.1"]);
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured((*tls).clone())
        .resolve(
            HOST,
            proving_ground.address().parse().expect("a socket address"),
        )
        .no_proxy()
        .build()
        .expect("a client");
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper builds the runtime its reqwest client runs on (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let response = client
            .get(format!("https://{HOST}:{}{path}", proving_ground.port()))
            .send()
            .await
            .expect("the Proving ground answers");
        let status = response.status().as_u16();
        let body = response.bytes().await.expect("a body");
        (
            status,
            String::from_utf8(body.to_vec()).expect("a UTF-8 body"),
        )
    })
}

/// The pinned Chrome for Testing build, in a Browser cache of the test's
/// folder, loading pages of one Proving ground.
struct PinnedBrowser<'a> {
    env: &'a TestEnv,
    proving_ground: &'a ProvingGround,
    chrome: std::path::PathBuf,
    switches: &'a [&'a str],
}

impl<'a> PinnedBrowser<'a> {
    fn new(env: &'a TestEnv, proving_ground: &'a ProvingGround, switches: &'a [&'a str]) -> Self {
        let cache = browser_cache(env).expect("the pinned build is installed");
        Self {
            env,
            proving_ground,
            chrome: chrome_for_testing(Path::new(&cache)),
            switches,
        }
    }

    /// Starts the browser, headless, on `path` over TLS; it prints the
    /// page's DOM once loaded, its scripts run, and exits.
    fn start(&self, path: &str) -> Process {
        let mut command = self.env.process_command(&self.chrome);
        command.args([
            "--headless",
            "--no-first-run",
            "--no-default-browser-check",
            "--password-store=basic",
            "--use-mock-keychain",
            // The Proving ground's authority is not the system's; skipping
            // its check changes nothing the ClientHello sends.
            "--ignore-certificate-errors",
        ]);
        command.args(self.switches);
        command.arg(format!(
            "--user-data-dir={}",
            self.env
                .folder()
                .join(self.env.unique_name("chrome-profile"))
                .display()
        ));
        command.arg(format!("--host-resolver-rules=MAP {HOST} 127.0.0.1"));
        command.arg("--dump-dom");
        command.arg(format!(
            "https://{HOST}:{}{path}",
            self.proving_ground.port()
        ));
        command.stderr(Stdio::null());
        self.env.spawn(&mut command).expect("Chrome starts")
    }

    /// The DOM a started browser printed.
    fn dom(browser: &Process) -> String {
        let mut dom = String::new();
        let stdout = browser.child().stdout.take().expect("stdout is piped");
        { stdout }
            .read_to_string(&mut dom)
            .expect("Chrome prints the DOM");
        let status = browser.close_stdin_and_wait().expect("Chrome exits");
        assert!(status.success(), "Chrome for Testing exited with {status}");
        dom
    }

    /// The DOM of `path` once loaded.
    fn loads(&self, path: &str) -> String {
        Self::dom(&self.start(path))
    }
}

/// The manifest's pages.
fn manifest(proving_ground: &ProvingGround) -> Vec<Value> {
    let answer = proving_ground.get(&proving_ground.url(HOST, "/manifest.json"));
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()["pages"]
        .as_array()
        .expect("the manifest lists its pages")
        .clone()
}

/// The manifest's Records of the page at `path`.
fn expected_records(proving_ground: &ProvingGround, path: &str) -> Vec<Value> {
    manifest(proving_ground)
        .iter()
        .find(|page| page["path"] == path)
        .unwrap_or_else(|| panic!("the manifest lists {path}"))["records"]
        .as_array()
        .expect("the page lists its Records")
        .clone()
}

/// A browser-identity page refuses reqwest, over TLS and over plain text,
/// with a `403` that holds no Record.
#[test]
fn a_browser_identity_page_refuses_reqwest() {
    let proving_ground = ProvingGround::start(SEED);

    let over_tls = reqwest_gets(&proving_ground, "/identity/1");
    let plain = proving_ground.get(&proving_ground.url(HOST, "/identity/1"));

    assert_eq!(
        (
            over_tls.0,
            records(&over_tls.1),
            plain.status,
            records(&plain.body)
        ),
        (403, vec![], 403, vec![])
    );
}

/// A browser-identity page answers its Record to the pinned browser,
/// started as Collie starts it or with no switch of Collie's.
#[rstest]
#[case::as_collie_starts_it(AS_COLLIE_STARTS_IT)]
#[case::with_its_field_trials(&[])]
fn a_browser_identity_page_answers_the_pinned_browser(#[case] switches: &[&str]) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(SEED);
    let browser = PinnedBrowser::new(&env, &proving_ground, switches);

    let read = records(&browser.loads("/identity/1"));

    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(read[0]["id"], "identity-1");
    assert_eq!(read, expected_records(&proving_ground, "/identity/1"));
}

/// A static page holds its Record in its own markup; a script-rendered
/// page's markup holds none, its item empty until its script fills it.
#[test]
fn a_static_page_s_markup_holds_its_record_and_a_script_rendered_page_s_none() {
    let proving_ground = ProvingGround::start(SEED);
    let markup = |path: &str| {
        let answer = proving_ground.get(&proving_ground.url(HOST, path));
        (answer.status, records(&answer.body))
    };

    let (status, in_static) = markup("/static/1");

    assert_eq!(
        (status, &in_static),
        (200, &expected_records(&proving_ground, "/static/1"))
    );
    assert_eq!(in_static[0]["id"], "static-1");
    assert_eq!(markup("/scripted/1"), (200, vec![]));
}

/// The manifest lists the home page and every page of each kind, with its
/// kind and one Record named after its path; no other path is a page.
#[test]
fn the_manifest_lists_every_page_with_its_kind_and_its_records() {
    let proving_ground = ProvingGround::start(SEED);

    let listed: Vec<(String, String, Vec<String>)> = manifest(&proving_ground)
        .iter()
        .map(|page| {
            let text = |value: &Value| value.as_str().expect("a string").to_owned();
            let ids = page["records"]
                .as_array()
                .expect("the page lists its Records")
                .iter()
                .map(|record| {
                    assert!(!text(&record["name"]).is_empty(), "{record}");
                    text(&record["id"])
                })
                .collect();
            (text(&page["path"]), text(&page["kind"]), ids)
        })
        .collect();

    let mut expected = vec![("/".to_owned(), "static".to_owned(), vec![])];
    for (segment, kind) in [
        ("static", "static"),
        ("identity", "browser_identity"),
        ("scripted", "script_rendered"),
    ] {
        for number in 1..=PAGES_PER_KIND {
            expected.push((
                format!("/{segment}/{number}"),
                kind.to_owned(),
                vec![format!("{segment}-{number}")],
            ));
        }
    }
    assert_eq!(listed, expected);
    let status = |path: &str| proving_ground.get(&proving_ground.url(HOST, path)).status;
    assert_eq!(
        [
            status("/static/01"),
            status("/static/7"),
            status("/static"),
            status("/other/1")
        ],
        [404; 4]
    );
}

/// A crawl by the pinned browser from the home page, following every link,
/// reads on each page exactly the Records the manifest lists for it, and
/// reaches every page the manifest lists and no other.
#[test]
fn a_pinned_browser_s_crawl_reads_exactly_the_manifest_s_records() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(SEED);
    let browser = PinnedBrowser::new(&env, &proving_ground, AS_COLLIE_STARTS_IT);

    let mut read: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut found = BTreeSet::from(["/".to_owned()]);
    loop {
        let next: Vec<String> = found
            .iter()
            .filter(|path| !read.contains_key(*path))
            .take(BROWSERS_AT_ONCE)
            .cloned()
            .collect();
        if next.is_empty() {
            break;
        }
        let started: Vec<Process> = next.iter().map(|path| browser.start(path)).collect();
        for (path, started) in next.into_iter().zip(&started) {
            let dom = PinnedBrowser::dom(started);
            found.extend(links(&dom));
            read.insert(path, records(&dom));
        }
    }

    let expected: BTreeMap<String, Vec<Value>> = manifest(&proving_ground)
        .iter()
        .map(|page| {
            (
                page["path"].as_str().expect("a path").to_owned(),
                page["records"].as_array().expect("its Records").clone(),
            )
        })
        .collect();
    assert_eq!(read, expected);
}
