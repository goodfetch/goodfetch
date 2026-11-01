//! The Console's content origin through Collie's executable (spec #592;
//! ADRs 0031, 0049 and 0067): the Console's origin gives a signed-in page a
//! Run's file by handle, as an attachment, and turns its handle into an
//! address on the content origin signed for that one file; the content
//! origin shows the file to that address alone, under a policy that runs
//! no script and lets only the Console frame it, and sets and reads no
//! cookie. Its refusals are the endpoint table's `content_origin` column,
//! each checked in the Audit log; behind a proxy it shares the Console's
//! listener, told apart by `Host`.

#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::net::SocketAddr;

use goodfetch_test_support::{
    AuditUnreadable, Endpoint, EndpointRunner, Listener, ListenerKind, Rule, TestEnv, cli,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::commands::{start, store_body};
use crate::console::policy;
use crate::daemon::StopsTheDaemon;
use crate::lasting_sign_ins::audit_entries;
use crate::sign_in::{agent, console_url, origin, runtime_file, sign_in};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// A hostile page's body, whose inline script must never run.
const PAGE: &[u8] =
    b"<!doctype html><title>Scraped</title><p>Scraped.</p><script>document.title = 'ran'</script>";

/// A started Daemon with a signed-in Console and a Run holding [`PAGE`].
pub(crate) struct Signed {
    /// The Console's origin, `http://127.0.0.1:<port>`.
    pub(crate) console: String,
    /// The sign-in's cookie pair, `name=value`.
    pub(crate) cookie: String,
    /// The body's ID in the Run `first`.
    pub(crate) body: String,
}

impl Signed {
    /// Starts `env`'s Daemon, signs in through the link and stores
    /// [`PAGE`] as a body of the Run `first`.
    pub(crate) fn started(env: &TestEnv) -> Self {
        let started = cli(env, COLLIE).args(["daemon", "start"]).run().unwrap();
        assert_eq!(started.exit, Some(0), "{}", started.stderr);
        start(env, "first");
        let body = store_body(env, "first", PAGE);
        let url = console_url(env);
        let (status, set) = sign_in(&url, &runtime_file(env).token);
        assert_eq!(status, 204);
        let cookie = set.unwrap().split("; ").next().unwrap().to_owned();
        Self {
            console: origin(&url).to_owned(),
            cookie,
            body,
        }
    }

    /// What the Console answers a signed-in `GET` of `path`: its status,
    /// headers and body.
    pub(crate) fn get(&self, path: &str) -> Answer {
        Answer::of(
            agent()
                .get(format!("{}{path}", self.console))
                .header("Cookie", &self.cookie)
                .call()
                .unwrap(),
        )
    }

    /// The content address of the body shown as `shown`.
    pub(crate) fn address(&self, shown: &str) -> String {
        let answer = self.get(&format!(
            "/content-address/first/{}?type={shown}",
            self.body
        ));
        assert_eq!(answer.status, 200, "{answer:?}");
        let value: Value = serde_json::from_slice(&answer.body).unwrap();
        value["url"].as_str().unwrap().to_owned()
    }

    /// The content origin, `http://127.0.0.1:<port>`.
    pub(crate) fn content(&self) -> String {
        let address = self.address("html");
        let (origin, _) = address.split_once("/bodies/").unwrap();
        origin.to_owned()
    }
}

/// An HTTP answer.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, Vec<String>>,
    pub(crate) body: Vec<u8>,
}

impl Answer {
    fn of(mut response: ureq::http::Response<ureq::Body>) -> Self {
        let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, value) in response.headers() {
            headers
                .entry(name.as_str().to_owned())
                .or_default()
                .push(value.to_str().unwrap().to_owned());
        }
        Self {
            status: response.status().as_u16(),
            headers,
            body: response.body_mut().read_to_vec().unwrap(),
        }
    }

    /// The one value of the header `name`, if any.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(|values| {
            assert_eq!(values.len(), 1, "{name}: {values:?}");
            values[0].as_str()
        })
    }
}

/// What `url` answers a `method` request carrying `headers`.
fn request(method: &str, url: &str, headers: &[(&str, &str)]) -> Answer {
    let agent = agent();
    let response = match method {
        "GET" => {
            let mut request = agent.get(url);
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            request.call()
        }
        "HEAD" => {
            let mut request = agent.head(url);
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            request.call()
        }
        _ => {
            let mut request = agent.post(url);
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            request.send_empty()
        }
    };
    Answer::of(response.unwrap())
}

/// `address` with its signature's first digit changed.
fn altered(address: &str) -> String {
    let (rest, signature) = address.rsplit_once("signature=").unwrap();
    let first = if signature.starts_with('0') { '1' } else { '0' };
    format!("{rest}signature={first}{}", &signature[1..])
}

/// The content origin's policy, framed only by the Console at `console`.
fn content_policy(console: &str) -> String {
    let port = console.rsplit_once(':').unwrap().1;
    format!(
        "sandbox; default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; form-action 'none'; frame-ancestors {console} http://localhost:{port}"
    )
}

/// Covers: story:12-127, command:daemon.start
#[test]
fn a_files_signed_address_shows_it_on_the_content_origin_and_sets_no_cookie() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);

    let address = signed.address("html");
    let shown = request("GET", &address, &[]);

    assert_eq!(shown.status, 200, "{shown:?}");
    assert_eq!(shown.body, PAGE);
    assert_eq!(
        shown.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(shown.header("set-cookie"), None);
    assert!(
        !address.starts_with(&signed.console),
        "the content origin is not the Console's: {address}"
    );
    let image = request("GET", &signed.address("png"), &[]);
    assert_eq!(image.header("content-type"), Some("image/png"));
    let with_the_cookie = request("GET", &altered(&address), &[("Cookie", &signed.cookie)]);
    assert_eq!(
        with_the_cookie.status, 401,
        "a sign-in's cookie is no credential there"
    );
}

/// Covers: story:12-127
#[test]
fn the_console_gives_a_runs_file_by_handle_only_to_a_sign_in() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);
    let path = format!("/bodies/first/{}", signed.body);

    let fetched = signed.get(&path);
    let unsigned = request("GET", &format!("{}{path}", signed.console), &[]);
    let no_address = request(
        "GET",
        &format!("{}/content-address/first/{}", signed.console, signed.body),
        &[],
    );
    let unknown = signed.get(&format!("/content-address/first/{}", "0".repeat(64)));

    assert_eq!(fetched.status, 200, "{fetched:?}");
    assert_eq!(fetched.body, PAGE);
    assert_eq!(
        fetched.header("content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(fetched.header("content-disposition"), Some("attachment"));
    assert_eq!(unsigned.status, 401);
    assert_eq!(no_address.status, 401);
    assert_eq!(unknown.status, 404);
}

/// What a row of the content origin's headers table asks for.
#[derive(Clone, Copy, Debug)]
enum Asked {
    /// The body's signed address.
    Signed,
    /// It, its signature altered.
    Altered,
    /// Its path, with no signature.
    Unsigned,
    /// A path that names no file.
    NoFile,
    /// The signed address, naming a foreign `Host`.
    ForeignHost,
}

/// Every answer of every route of the content origin carries its headers:
/// its policy (a sandbox, no script, framed by the Console alone), no
/// referrer, no sniffing, nothing stored, and no cookie.
///
/// Covers: story:12-129, story:12-127
#[rstest]
#[case::the_file("GET", Asked::Signed, 200, Some("text/html; charset=utf-8"))]
#[case::its_head("HEAD", Asked::Signed, 200, Some("text/html; charset=utf-8"))]
#[case::an_altered_address("GET", Asked::Altered, 401, None)]
#[case::an_unsigned_address("GET", Asked::Unsigned, 401, None)]
#[case::a_path_with_no_file("GET", Asked::NoFile, 404, None)]
#[case::a_write("POST", Asked::Signed, 405, None)]
#[case::a_foreign_host("GET", Asked::ForeignHost, 403, None)]
fn every_content_origin_answer_carries_its_headers(
    #[case] method: &str,
    #[case] asked: Asked,
    #[case] status: u16,
    #[case] content_type: Option<&str>,
) {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);
    let address = signed.address("html");
    let content = signed.content();
    let (url, headers): (String, &[(&str, &str)]) = match asked {
        Asked::Signed => (address, &[]),
        Asked::Altered => (altered(&address), &[]),
        Asked::Unsigned => (address.split_once('?').unwrap().0.to_owned(), &[]),
        Asked::NoFile => (format!("{content}/runs"), &[]),
        Asked::ForeignHost => (address, &[("Host", "foreign.invalid")]),
    };

    let answer = request(method, &url, headers);

    assert_eq!(answer.status, status, "{answer:?}");
    assert_eq!(
        answer.header("content-security-policy"),
        Some(content_policy(&signed.console).as_str())
    );
    assert_eq!(answer.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(answer.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        answer.header("cross-origin-opener-policy"),
        Some("same-origin")
    );
    assert_eq!(
        answer.header("cross-origin-resource-policy"),
        Some("same-site")
    );
    assert_eq!(answer.header("cache-control"), Some("private, no-store"));
    assert_eq!(answer.header("content-type"), content_type);
    assert_eq!(answer.header("set-cookie"), None);
}

/// The endpoint table's reason the content origin records each rule's
/// refusal with: it reads no cookie, so the Console's is no token there.
fn reason(rule: Rule) -> &'static str {
    match rule {
        Rule::TokenMissing | Rule::TokenFromAnotherListener => "token_missing",
        Rule::HostForeign => "host_foreign",
        Rule::OriginForeign => "origin_foreign",
        Rule::TokenWrong
        | Rule::TokenForAnotherFile
        | Rule::TokenExpired
        | Rule::TokenRevoked
        | Rule::TokenScopedElsewhere
        | Rule::TokenInQueryString
        | Rule::OriginMissing
        | Rule::OriginMissingWithoutHeaderToken
        | Rule::HelloNotFirst
        | Rule::ProbesAnswerStatusOnly
        | Rule::CsrfTokenMissing
        | Rule::SignInRevoked
        | Rule::SignInRateExceeded => "token_wrong",
    }
}

/// The content origin, as the endpoint table's column: it passes every
/// row, each refusal recorded in the Audit log with the listener and the
/// rule's reason.
///
/// Covers: story:12-129, story:12-127
#[test]
fn the_content_origin_passes_its_endpoint_table_rows() {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);
    let other = store_body(&env, "first", b"another file");
    let address = signed.address("html");
    let authority = signed.content().trim_start_matches("http://").to_owned();
    let socket: SocketAddr = authority.parse().unwrap();
    let target = address.trim_start_matches(&format!("http://{authority}"));
    let listener = Listener::new(
        ListenerKind::ContentOrigin,
        Endpoint::SignedAddress {
            address: socket,
            host: authority.clone(),
            other_file: format!("/bodies/first/{other}"),
        },
        target,
    )
    .with_other_listener_token(signed.cookie.clone());
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    let verifier = |_: ListenerKind, rule: Rule| {
        let reason = reason(rule);
        let refusals = audit_entries(&env)
            .iter()
            .filter(|entry| {
                entry["kind"] == json!("endpoint_authentication_failed")
                    && entry["details"] == json!({ "listener": "content_origin", "reason": reason })
            })
            .count();
        let earlier = seen.insert(reason, refusals).unwrap_or(0);
        Ok::<_, AuditUnreadable>(refusals > earlier)
    };

    let ran = EndpointRunner::new(verifier).run(&listener);

    assert_eq!(ran, Ok(()));
}

/// Behind a proxy, with `console.content_public_url` and no content
/// address of its own, the content origin shares the Console's listener:
/// a request naming its host is the content origin's, and the addresses
/// the Console signs name it.
///
/// Covers: story:12-127, setting:console.content_public_url
#[test]
fn behind_a_proxy_the_content_origin_shares_the_consoles_listener_by_host() {
    let env = TestEnv::new();
    env.user_configuration(
        "console.address = \"127.0.0.1:0\"\nconsole.content_public_url = \"https://content.example.test\"\n",
    );
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);

    let address = signed.address("html");
    let target = address
        .strip_prefix("https://content.example.test")
        .expect("the address names the public origin");
    let shown = request(
        "GET",
        &format!("{}{target}", signed.console),
        &[("Host", "content.example.test")],
    );
    let as_the_console = request("GET", &format!("{}{target}", signed.console), &[]);

    assert_eq!(shown.status, 200, "{shown:?}");
    assert_eq!(shown.body, PAGE);
    assert_eq!(
        as_the_console.status, 401,
        "the Console's own `/bodies/` takes a sign-in, not an address"
    );
    let policy = as_the_console.header("content-security-policy").unwrap();
    assert!(
        policy.contains("frame-src https://content.example.test;"),
        "{policy}"
    );
}

/// A content address another program holds leaves the Console serving,
/// with no content origin: it signs no address.
///
/// Covers: story:12-127, setting:console.content_address
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "a test holds a port the content origin then finds taken, from outside the engine (ADR 0060)"
)]
fn a_content_address_another_program_holds_leaves_the_console_without_it() {
    let env = TestEnv::new();
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = held.local_addr().unwrap();
    env.user_configuration(&format!(
        "console.address = \"127.0.0.1:0\"\nconsole.content_address = \"{taken}\"\n"
    ));
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);

    let asked = signed.get(&format!("/content-address/first/{}", signed.body));
    let page = signed.get("/");

    assert_eq!(asked.status, 503);
    assert_eq!(page.status, 200);
    drop(held);
}

/// A `console.content_public_url` that is no bare `http` or `https` origin
/// (a wildcard, a path, a user, a second directive) leaves the Console
/// serving without a content origin: its policy frames nothing, instead of
/// framing more than one origin or carrying the text as a directive.
///
/// Covers: story:12-127, setting:console.content_public_url
#[rstest]
#[case::a_wildcard("https://*.example.test")]
#[case::another_directive("https://content.example.test; script-src *")]
#[case::a_path("https://content.example.test/files")]
#[case::a_query("https://content.example.test?x=1")]
#[case::a_user("https://user@content.example.test")]
#[case::a_scheme_of_no_origin("ftp://content.example.test")]
#[case::a_space("https://content.example.test https://other.example.test")]
#[case::an_empty_port("https://content.example.test:")]
fn a_public_url_that_is_no_origin_leaves_the_console_without_a_content_origin(#[case] url: &str) {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "console.address = \"127.0.0.1:0\"\nconsole.content_public_url = \"{url}\"\n"
    ));
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);

    let asked = signed.get(&format!("/content-address/first/{}", signed.body));
    let page = signed.get("/");

    assert_eq!(asked.status, 503, "{asked:?}");
    assert_eq!(page.status, 200);
    assert_eq!(
        page.header("content-security-policy"),
        Some(policy(None).as_str())
    );
}

/// A `console.content_public_url` that is a bare origin, with or without a
/// port or a closing slash, is the one origin the Console's policy frames.
///
/// Covers: story:12-127, setting:console.content_public_url
#[rstest]
#[case::with_a_port(
    "https://content.example.test:8443",
    "https://content.example.test:8443"
)]
#[case::with_a_closing_slash("https://content.example.test/", "https://content.example.test")]
#[case::on_http("http://content.example.test", "http://content.example.test")]
#[case::an_ipv6_address("http://[::1]:8443/", "http://[::1]:8443")]
fn a_public_url_that_is_an_origin_is_the_one_origin_the_console_frames(
    #[case] url: &str,
    #[case] framed: &str,
) {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "console.address = \"127.0.0.1:0\"\nconsole.content_public_url = \"{url}\"\n"
    ));
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let signed = Signed::started(&env);

    let page = signed.get("/");

    assert_eq!(
        page.header("content-security-policy"),
        Some(policy(Some(framed)).as_str())
    );
}
