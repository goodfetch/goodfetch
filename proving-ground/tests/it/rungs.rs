//! The rung site on `rungs.localhost`: pages of three kinds, each needing
//! one rung of `auto`'s ladder, and two that refuse part way. A static page
//! is the same to every client; a browser-identity page answers its content
//! only to the TLS fingerprint of the Chrome for Testing build Collie pins,
//! and refuses any other client; a script-rendered page holds its Record
//! only once its script ran. A transport-refused page is static up to page
//! 3, then a browser-identity page whose refusal says the address is
//! blocked; an every-client-refused page gives that refusal to every
//! client from page 4. Its manifest lists every page with its kind and its expected
//! Records. It belongs to no task and declares no Feature.
//!
//! The pinned browser here is the build test support lays out in a
//! Browser cache, started once per page with `--dump-dom`, never an
//! engine crate.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::net::SocketAddr;
use std::path::PathBuf;

use goodfetch_test_support::{Process, TestEnv, browser_cache, chrome_for_testing_dumping_dom};
use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::clients;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 2410;

const HOST: &str = "rungs.localhost";

/// The switch Collie starts its browsers with that changes their TLS
/// fingerprint: without the field-trial configuration built into the
/// binary, Chrome sends one extension fewer.
pub(crate) const AS_COLLIE_STARTS_IT: &[&str] = &["--disable-field-trial-config"];

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

/// What reqwest gets for `path` over TLS, on a rustls configuration with
/// the ring provider, as the HTTP fetch tier's is: the status and the body.
fn reqwest_gets(proving_ground: &ProvingGround, path: &str) -> (u16, String) {
    reqwest_gets_on(proving_ground, HOST, path)
}

/// As [`reqwest_gets`], from `host`.
pub(crate) fn reqwest_gets_on(
    proving_ground: &ProvingGround,
    host: &str,
    path: &str,
) -> (u16, String) {
    let tls = clients::trusting(&proving_ground.certificate_authority(), &["h2", "http/1.1"]);
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured((*tls).clone())
        .resolve(
            host,
            proving_ground.address().parse().expect("a socket address"),
        )
        .no_proxy()
        .build()
        .expect("a client");
    let runtime = goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let response = client
            .get(format!("https://{host}:{}{path}", proving_ground.port()))
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
/// folder, loading pages of one host of one Proving ground.
pub(crate) struct PinnedBrowser<'a> {
    env: &'a TestEnv,
    proving_ground: &'a ProvingGround,
    cache: PathBuf,
    switches: &'a [&'a str],
    host: &'a str,
}

impl<'a> PinnedBrowser<'a> {
    fn new(env: &'a TestEnv, proving_ground: &'a ProvingGround, switches: &'a [&'a str]) -> Self {
        Self::on(env, proving_ground, switches, HOST)
    }

    /// The browser, loading pages of `host`.
    pub(crate) fn on(
        env: &'a TestEnv,
        proving_ground: &'a ProvingGround,
        switches: &'a [&'a str],
        host: &'a str,
    ) -> Self {
        Self {
            env,
            proving_ground,
            cache: browser_cache(env).expect("the pinned build is installed"),
            switches,
            host,
        }
    }

    /// Starts the browser on `path` over TLS; it prints the page's DOM once
    /// loaded, its scripts run, and exits.
    fn start(&self, path: &str) -> Process {
        let address: SocketAddr = self
            .proving_ground
            .address()
            .parse()
            .expect("a socket address");
        chrome_for_testing_dumping_dom(
            self.env,
            &self.cache,
            (self.host, address.ip()),
            self.switches,
            &format!("https://{}:{}{path}", self.host, address.port()),
        )
        .expect("Chrome starts")
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
    pub(crate) fn loads(&self, path: &str) -> String {
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

/// A refusing page answers reqwest its Record up to page 3, then a `403`
/// that says the visitor's address is blocked and holds no Record; a
/// transport-refused page answers that Record to the pinned browser.
#[test]
fn a_refusing_page_refuses_reqwest_from_page_4_saying_its_address_is_blocked() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(SEED);
    let browser = PinnedBrowser::new(&env, &proving_ground, AS_COLLIE_STARTS_IT);
    let read = |path: &str| {
        let (status, body) = reqwest_gets(&proving_ground, path);
        let blocked = body.contains("Your IP address has been blocked.");
        (status, records(&body).len(), blocked)
    };

    assert_eq!(
        [
            read("/refused/3"),
            read("/refused/4"),
            read("/blocked/3"),
            read("/blocked/9"),
        ],
        [
            (200, 1, false),
            (403, 0, true),
            (200, 1, false),
            (403, 0, true)
        ]
    );
    assert_eq!(
        records(&browser.loads("/refused/4")),
        expected_records(&proving_ground, "/refused/4")
    );
    let blocked = browser.loads("/blocked/4");
    assert_eq!(
        (
            records(&blocked).len(),
            blocked.contains("Your IP address has been blocked.")
        ),
        (0, true)
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

/// Every page tells the visitors that are no browser apart by a cookie: a
/// request that sends no `visitor` cookie is answered one of its own, a
/// number no visitor was given before, refused or not; one that sends it
/// is answered none, and neither are the manifest and a path the site does
/// not have. That the pinned browser is given none, Foxhound's code-free
/// crawl of the site shows: its browser Run is refused nothing.
#[test]
fn every_page_gives_a_new_visitor_a_cookie_of_its_own() {
    let proving_ground = ProvingGround::start(SEED);
    let given = |path: &str, cookie: &[(&str, &str)]| {
        proving_ground
            .get_with_headers(&proving_ground.url(HOST, path), cookie)
            .header("set-cookie")
            .map(str::to_owned)
    };

    assert_eq!(
        [
            given("/static/1", &[]),
            given("/identity/1", &[]),
            given("/static/2", &[("cookie", "visitor=1")]),
            given("/manifest.json", &[]),
            given("/other/1", &[]),
            given("/", &[]),
        ],
        [
            Some("visitor=1; Path=/; SameSite=Lax".to_owned()),
            Some("visitor=2; Path=/; SameSite=Lax".to_owned()),
            None,
            None,
            None,
            Some("visitor=3; Path=/; SameSite=Lax".to_owned()),
        ]
    );
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

    let expected = [
        ("/", "static", None),
        ("/static/1", "static", Some("static-1")),
        ("/static/2", "static", Some("static-2")),
        ("/static/3", "static", Some("static-3")),
        ("/static/4", "static", Some("static-4")),
        ("/static/5", "static", Some("static-5")),
        ("/static/6", "static", Some("static-6")),
        ("/identity/1", "browser_identity", Some("identity-1")),
        ("/identity/2", "browser_identity", Some("identity-2")),
        ("/identity/3", "browser_identity", Some("identity-3")),
        ("/identity/4", "browser_identity", Some("identity-4")),
        ("/identity/5", "browser_identity", Some("identity-5")),
        ("/identity/6", "browser_identity", Some("identity-6")),
        ("/scripted/1", "script_rendered", Some("scripted-1")),
        ("/scripted/2", "script_rendered", Some("scripted-2")),
        ("/scripted/3", "script_rendered", Some("scripted-3")),
        ("/scripted/4", "script_rendered", Some("scripted-4")),
        ("/scripted/5", "script_rendered", Some("scripted-5")),
        ("/scripted/6", "script_rendered", Some("scripted-6")),
        ("/refused/1", "transport_refused", Some("refused-1")),
        ("/refused/2", "transport_refused", Some("refused-2")),
        ("/refused/3", "transport_refused", Some("refused-3")),
        ("/refused/4", "transport_refused", Some("refused-4")),
        ("/refused/5", "transport_refused", Some("refused-5")),
        ("/refused/6", "transport_refused", Some("refused-6")),
        ("/refused/7", "transport_refused", Some("refused-7")),
        ("/refused/8", "transport_refused", Some("refused-8")),
        ("/refused/9", "transport_refused", Some("refused-9")),
        ("/blocked/1", "every_client_refused", Some("blocked-1")),
        ("/blocked/2", "every_client_refused", Some("blocked-2")),
        ("/blocked/3", "every_client_refused", Some("blocked-3")),
        ("/blocked/4", "every_client_refused", None),
        ("/blocked/5", "every_client_refused", None),
        ("/blocked/6", "every_client_refused", None),
        ("/blocked/7", "every_client_refused", None),
        ("/blocked/8", "every_client_refused", None),
        ("/blocked/9", "every_client_refused", None),
    ]
    .map(|(path, kind, id)| {
        (
            path.to_owned(),
            kind.to_owned(),
            id.into_iter().map(str::to_owned).collect::<Vec<_>>(),
        )
    });
    assert_eq!(listed, expected);
    let status = |path: &str| proving_ground.get(&proving_ground.url(HOST, path)).status;
    assert_eq!(
        [
            status("/static/01"),
            status("/static/7"),
            status("/refused/10"),
            status("/static"),
            status("/other/1")
        ],
        [404; 5]
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
