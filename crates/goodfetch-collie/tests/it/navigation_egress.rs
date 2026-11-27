//! A page's navigations under the egress policy over stdio (spec #563's
//! stories 5 and 6; ADRs 0017, 0056, 0089 and 0120), against the Proving
//! ground's redirect pages, whose connection record is the oracle: every
//! hop of a redirect of any status or kind is judged again, one to a
//! scheme other than HTTP's is refused with `egress_refused`, a `file:`
//! URL is loaded only for the program's own code, and a `javascript:` URL
//! runs only in a frame whose origin the domain policy allows.
//!
//! The browser layer's resolved-address check is the egress relay's: the
//! SSRF table's redirects to a private address are refused at the relay,
//! where their hop connects, and raise `egress_refused` naming the address,
//! behind an `https://` proxy too, which the relay chains through inside TLS
//! (issue 1609). A launched Session with no relay under it, with
//! `security.raw_cdp_egress_proxy` off, is held by interception alone,
//! which resolves each host in the engine and judges every answer (#2435).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests poll the Proving ground and the page on their own thread, never an async worker"
)]

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::robots_txt_read;
use goodfetch_test_support::{
    Answer, BROWSERS_PATH_VARIABLE, Outcome, ProvingGround, Refusal, Route, Runner, SsrfCase,
    SsrfTable, TestEnv, browser_cache,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::COLLIE;
use crate::egress_relay::provenance;
use crate::page::{BROWSER_DEADLINE, opened};
use goodfetch_test_client::{Attached, Hello};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 631;

/// The host the redirect family's stand-ins are reached on: any of the
/// family's hosts serves every case's page.
const STAND_INS: &str = "1.redirects.test";

/// The host a case's URL names for its public stand-in.
const CASE_HOST: &str = "http://redirect.ssrf.test/";

/// How long a case waits for a hop after its page loaded: a refresh or a
/// script redirects once the page is in.
const HOP_DEADLINE: Duration = Duration::from_secs(10);

/// A layer of the SSRF table and the Provenance entries each of its Runs
/// records of its Session's egress, as `(egress, gap)`.
struct Layer {
    name: &'static str,
    gap: &'static [(&'static str, &'static str)],
}

/// The layer of a launched Session with its egress relay, which records no
/// gap.
const RELAYED: Layer = Layer {
    name: "chromium_navigation",
    gap: &[],
};

/// The layer of a launched Session with no egress relay (#2435), held by
/// interception alone, which records the rebinding gap.
const WITHOUT_RELAY: Layer = Layer {
    name: "launched_without_relay",
    gap: &[("interception_only", "dns_rebinding")],
};

/// How a launched Session's connections leave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Egress {
    /// Through its egress relay, the default.
    Relay,
    /// Directly, with `security.raw_cdp_egress_proxy` off.
    SwitchedOff,
    /// Through its egress relay chained through an `https://` proxy (issue
    /// 1609): the Proving ground's `open_tls` proxy, at its name, whose
    /// authority the user configuration trusts for it.
    HttpsProxy,
}

impl Egress {
    /// Sets this egress in `settings`, a Run's, `tls_proxy` the Proving
    /// ground's `https://` proxy's name and port.
    fn apply(self, settings: &mut Value, tls_proxy: &str) {
        match self {
            Self::Relay => {}
            Self::SwitchedOff => {
                settings["security"] = json!({ "raw_cdp_egress_proxy": false });
            }
            Self::HttpsProxy => {
                settings["proxy"] = json!({ "server": format!("https://{tls_proxy}") });
            }
        }
    }

    /// The Proving ground proxy a navigation reaches when it goes on: the
    /// `https://` one behind it, else the open one an upstream proxy case
    /// names.
    fn proxy(self) -> &'static str {
        match self {
            Self::Relay | Self::SwitchedOff => "open",
            Self::HttpsProxy => "open_tls",
        }
    }
}

/// The name the Proving ground's `https://` proxy's leaf names, which its
/// hosts map to its address.
const TLS_PROXY_HOST: &str = "tls-proxy.test";

/// The user configuration trusting the Proving ground's authority for its
/// `https://` proxy, as an Operator configures a proxy's own CA.
fn trusting_the_tls_proxy(env: &TestEnv, proving_ground: &ProvingGround) {
    let ca = proving_ground.description().ca.display().to_string();
    env.user_configuration(&format!(
        "tls.trusted_cas = {{ \"{TLS_PROXY_HOST}\" = [{}] }}\n",
        json!(ca)
    ));
}

/// The Proving ground's `https://` proxy as `proxy.server` names it: its
/// name and port.
fn tls_proxy(proving_ground: &ProvingGround) -> String {
    let address: std::net::SocketAddr = proving_ground.description().fixtures.proxies["open_tls"]
        .parse()
        .unwrap();
    format!("{TLS_PROXY_HOST}:{}", address.port())
}

/// A Connection to an Attached engine whose `hello` names `surface`, and
/// the Proving ground's hosts mapped in each Run's `network.resolve`,
/// which a program's Run lists in `egress.private_ranges`, since the relay
/// and the interception judge a mapping's answers as any other; a Client
/// acting for a model lists none. Each Run's Sessions leave as `egress`
/// says.
struct Program {
    engine: Attached,
    next_id: i64,
    resolve: Map<String, Value>,
    private_ranges: Vec<String>,
    for_model: bool,
    egress: Egress,
    tls_proxy: String,
}

impl Program {
    fn start(
        env: &TestEnv,
        cache: &Path,
        proving_ground: &ProvingGround,
        surface: Surface,
    ) -> Self {
        let mut engine = Attached::start_with(env, COLLIE, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
        });
        engine.send(&Hello::needing(Product::Collie).surface(surface).message());
        let _ = engine.next();
        let resolve = proving_ground
            .description()
            .resolve
            .iter()
            .map(|(host, address)| (host.clone(), json!([address])))
            .collect();
        Self {
            engine,
            next_id: 0,
            private_ranges: proving_ground
                .description()
                .resolve
                .keys()
                .cloned()
                .collect(),
            resolve,
            for_model: surface == Surface::Mcp,
            egress: Egress::Relay,
            tls_proxy: tls_proxy(proving_ground),
        }
    }

    /// The same, its Runs' Sessions leaving as `egress` says.
    fn leaving(self, egress: Egress) -> Self {
        Self { egress, ..self }
    }

    /// Maps `host` to `addresses` in the Runs it starts from now on, as a
    /// case's lookups answer, without listing it in their private ranges.
    fn map(&mut self, host: &str, addresses: &[String]) {
        self.resolve.insert(host.to_owned(), json!(addresses));
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        self.engine.ask(self.next_id, method, params)
    }

    /// Starts the Run `run_id` whose domain policy allows `allowed`.
    fn run(&mut self, run_id: &str, allowed: &[&str]) {
        let mut settings = json!({
            "network": { "resolve": self.resolve },
            "egress": { "allowed_domains": allowed },
        });
        if !self.for_model {
            settings["egress"]["private_ranges"] = json!(self.private_ranges);
        }
        self.egress.apply(&mut settings, &self.tls_proxy);
        let started = self.call(
            "run.start",
            &json!({ "run_id": run_id, "settings": settings }),
        );
        assert!(started.get("result").is_some(), "{started}");
    }

    fn new_page(&mut self, run_id: &str) -> String {
        opened(&self.call("run.new_page", &json!({ "run_id": run_id })))
            .page_id
            .0
    }

    fn goto(&mut self, page: &str, url: &str) -> Value {
        self.call("page.goto", &json!({ "page_id": page, "url": url }))
    }

    /// The page's main content as Markdown, or the call's error.
    fn markdown(&mut self, page: &str) -> Result<String, Value> {
        let read = self.call(
            "page.formats",
            &json!({ "page_id": page, "formats": { "markdown": true } }),
        );
        match read["result"]["markdown"].as_str() {
            Some(markdown) => Ok(markdown.to_owned()),
            None => Err(read),
        }
    }

    fn finish(mut self, runs: &[&str]) {
        for run_id in runs {
            let _ = self.call("run.finish", &json!({ "run_id": run_id }));
        }
        let _ = self.engine.finish();
    }
}

/// The Error code of a call's error and its details.
fn error(answer: &Value) -> (Value, Value) {
    let mut details = answer["error"]["data"].clone();
    let code = details["code"].take();
    if let Some(details) = details.as_object_mut() {
        details.remove("code");
        details.remove("retryable");
    }
    (code, details)
}

/// The requests the Proving ground received that a navigation's hop may
/// have sent, as `(host, target)`: neither the browser's own for an icon
/// nor the engine's robots.txt read of a loaded document's origin, which
/// starts once the document loaded and which the navigation never waits
/// for (ADR 0086), so it may reach the site before or after the navigation
/// answers.
fn requests(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .map(|request| (request.host.clone(), request.target.clone()))
        .collect()
}

/// How many requests for a page, not the browser's own for an icon, the
/// Proving ground received: a robots.txt read included, so a count taken
/// after [`robots_txt_read`] still sees one a later navigation started.
fn pages_requested(proving_ground: &ProvingGround) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.target != "/favicon.ico")
        .count()
}

/// Covers: story:4-5, error:egress_refused, operation:page.goto
#[test]
fn a_chain_whose_third_hop_leaves_the_policy_is_refused_at_that_hop() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground, Surface::SdkPython);
    let start = proving_ground.url(STAND_INS, "/leaves/3");
    let target = proving_ground.url("catalog.test", "/catalog/1");
    program.run("held", &["redirects.test"]);
    program.run("open", &["*"]);
    let held = program.new_page("held");
    let open = program.new_page("open");

    let refused = program.goto(&held, &start);
    let held_requests = requests(&proving_ground);
    let followed = program.goto(&open, &start);
    let open_requests = requests(&proving_ground);
    program.finish(&["held", "open"]);

    // The browser's own request for the page's icon and the engine's
    // robots.txt read of the document's origin may come before the `goto`
    // answers, so only the page's own requests count.
    let reached = |requests: &[(String, String)]| {
        requests
            .iter()
            .filter(|(host, _)| host == "catalog.test")
            .count()
    };
    assert_eq!(
        (
            error(&refused),
            reached(&held_requests),
            &followed["result"]["response"]["url"],
            reached(&open_requests),
        ),
        (
            (
                json!("egress_refused"),
                json!({ "url": target, "rule": "domain_policy", "setting": "egress.allowed_domains" })
            ),
            0,
            &json!(target),
            1,
        )
    );
}

/// The host, without its port, and the path of an `http:` URL.
fn host_and_path(url: &str) -> (String, String) {
    let rest = url.strip_prefix("http://").unwrap();
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let host = authority.split(':').next().unwrap_or_default();
    (host.to_owned(), path.to_owned())
}

/// What a case's page did on Chromium: the refusal its navigation or the
/// page's next call raised, and the Proving ground's requests for anything
/// but the case's own page, which a hop reached, by the Proving ground's
/// address.
fn run_case(
    program: &mut Program,
    proving_ground: &ProvingGround,
    run_id: &str,
    case: &SsrfCase,
) -> Outcome {
    let port = proving_ground.address().port();
    let url = match case.url.strip_prefix(CASE_HOST) {
        Some(_) => proving_ground.url(STAND_INS, &format!("/{}", case.name)),
        None => case.url.replace("PORT", &port.to_string()),
    };
    let own = host_and_path(&url);
    let before = requests(proving_ground).len();
    let page = program.new_page(run_id);
    let reached = || -> Vec<IpAddr> {
        requests(proving_ground)
            .into_iter()
            .skip(before)
            .filter(|(host, target)| !(*host == own.0 && *target == own.1))
            .map(|_| proving_ground.address().ip())
            .collect()
    };
    let refusal = |answer: &Value| match error(answer) {
        (code, details) if code == "egress_refused" => {
            serde_json::from_value(details["rule"].clone())
                .ok()
                .map(|rule| Refusal::EgressRefused { rule })
        }
        _ => None,
    };
    let went = program.goto(&page, &url);
    if went.get("error").is_some() {
        return Outcome {
            refusal: refusal(&went),
            connections: reached(),
        };
    }
    // A refresh or a script hops once the page is in: its refusal is the
    // page's next call's, and a hop that went through reaches the site.
    let until = Instant::now() + HOP_DEADLINE;
    while Instant::now() < until {
        if let Err(answer) = program.markdown(&page)
            && let Some(refused) = refusal(&answer)
        {
            return Outcome {
                refusal: Some(refused),
                connections: reached(),
            };
        }
        let connections = reached();
        if !connections.is_empty() {
            return Outcome::connected(connections);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Outcome::connected(reached())
}

/// The SSRF table's thin runner on Chromium (ADR 0067) for the redirect
/// family, `test` the runner's test on `layer`, its Sessions leaving as
/// `egress` says: each case's page as the Proving ground serves it, every
/// status and kind of redirect and every scheme other than HTTP's, a hop to
/// a private address or to a name that resolves to one refused by the
/// egress relay, or with no relay by the interception, which judges each
/// hop as a paused request, the name's answer given by the Run's
/// `network.resolve` mapping, judged as any other. Positive controls: the
/// same kinds to an allowed page go through.
fn redirects_are_refused_on_chromium(layer: &Layer, test: &str, egress: Egress) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let public = proving_ground.address().ip();
    let runner = Runner::committed(layer.name, file!(), test).unwrap();
    let cases = runner.cases();
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "status_to_an_allowed_host"
        family = "redirect"
        url = "http://1.redirects.test:PORT/leaves/1"
        target = "public"
        domains = { allowed = ["redirects.test", "catalog.test"] }
        refusal = { code = "egress_refused", rule = "domain_policy" }

        [[cases]]
        name = "meta_refresh_to_an_allowed_page"
        family = "redirect"
        url = "http://fetch-edge.test:PORT/refresh/meta/0"
        target = "public"
        refusal = { code = "egress_refused", rule = "scheme" }

        [[cases]]
        name = "script_to_an_allowed_page"
        family = "redirect"
        url = "http://fetch-edge.test:PORT/refresh/script"
        target = "public"
        refusal = { code = "egress_refused", rule = "scheme" }
        "#,
    )
    .unwrap();
    let mut program =
        Program::start(&env, &cache, &proving_ground, Surface::SdkPython).leaving(egress);
    // A hop to a name the table resolves privately is judged with the
    // name's answer from the Run's mapping, which no range lists.
    for case in cases {
        for lookup in case.lookups.iter().skip(1) {
            let addresses: Vec<String> = lookup
                .answers
                .iter()
                .filter_map(|answer| match answer {
                    Answer::Address(address) => Some(address.to_string()),
                    Answer::Public => None,
                })
                .collect();
            program.map(&lookup.name, &addresses);
        }
    }
    // One Run per domain policy, which its pages share.
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut run_for = |program: &mut Program, case: &SsrfCase| -> String {
        let allowed = case
            .domains
            .as_ref()
            .map_or_else(|| vec!["*".to_owned()], |domains| domains.allowed.clone());
        let index = runs
            .iter()
            .position(|run| *run == allowed)
            .unwrap_or_else(|| {
                let allowed_hosts: Vec<&str> = allowed.iter().map(String::as_str).collect();
                program.run(&format!("policy-{}", runs.len()), &allowed_hosts);
                runs.push(allowed.clone());
                runs.len() - 1
            });
        format!("policy-{index}")
    };

    let result = runner.run(&controls.cases, public, |case| {
        let run_id = run_for(&mut program, case);
        run_case(&mut program, &proving_ground, &run_id, case)
    });
    let finished: Vec<String> = (0..runs.len()).map(|run| format!("policy-{run}")).collect();
    let run_ids: Vec<&str> = finished.iter().map(String::as_str).collect();
    program.finish(&run_ids);

    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
    assert_gap_recorded(&env, &finished, layer);
}

/// Each of the Runs `run_ids` recorded in Provenance, for its one Session,
/// the gap entries of `layer`.
fn assert_gap_recorded(env: &TestEnv, run_ids: &[String], layer: &Layer) {
    for run_id in run_ids {
        let entries = provenance(env, run_id, "interception_only");
        let gaps: Vec<(&str, &str)> = entries
            .iter()
            .map(|entry| {
                (
                    entry["egress"].as_str().unwrap_or_default(),
                    entry["gap"].as_str().unwrap_or_default(),
                )
            })
            .collect();
        assert_eq!(gaps, layer.gap, "{run_id}");
    }
}

/// The redirect cases on a launched Session with its relay: see
/// [`redirects_are_refused_on_chromium`].
///
/// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused
#[test]
fn every_redirect_case_of_the_ssrf_table_is_refused_at_its_hop_on_chromium() {
    redirects_are_refused_on_chromium(
        &RELAYED,
        "every_redirect_case_of_the_ssrf_table_is_refused_at_its_hop_on_chromium",
        Egress::Relay,
    );
}

/// The redirect cases on a launched Session with
/// `security.raw_cdp_egress_proxy` off, so no relay: see
/// [`redirects_are_refused_on_chromium`].
///
/// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused, setting:security.raw_cdp_egress_proxy
#[test]
fn every_redirect_case_of_the_ssrf_table_is_refused_at_its_hop_on_a_launched_session_with_no_relay()
{
    redirects_are_refused_on_chromium(
        &WITHOUT_RELAY,
        "every_redirect_case_of_the_ssrf_table_is_refused_at_its_hop_on_a_launched_session_with_no_relay",
        Egress::SwitchedOff,
    );
}

/// How many connections the Proving ground's listener `name` accepted, such
/// as its open proxy's.
fn listener_connections(proving_ground: &ProvingGround, name: &str) -> usize {
    proving_ground.observations().unwrap()["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|connection| connection["listener"] == json!(name))
        .count()
}

/// `case`'s URL as the program navigates to it: at the Proving ground's
/// port, so a navigation let through to its address would reach it, or as
/// the case writes it when the WHATWG parser refuses it.
fn navigated_url(case: &SsrfCase, port: u16) -> String {
    match url::Url::parse(&case.url) {
        Ok(mut url) => {
            let _ = url.set_port(Some(port));
            url.to_string()
        }
        Err(_) => case.url.clone(),
    }
}

/// The Run settings a case's navigation goes under: its domain policy, its
/// lookups in the Run's `network.resolve` mapping, `public` the Proving
/// ground's address, the names its positive controls resolve listed in
/// `egress.private_ranges`, and the Proving ground's open proxy for an
/// upstream proxy case.
fn case_settings(
    program: &Program,
    proving_ground: &ProvingGround,
    case: &SsrfCase,
    listed: &[String],
) -> Value {
    let public = proving_ground.address().ip().to_string();
    let mut resolve = program.resolve.clone();
    for lookup in &case.lookups {
        let answers: Vec<String> = lookup
            .answers
            .iter()
            .map(|answer| match answer {
                Answer::Public => public.clone(),
                Answer::Address(address) => address.to_string(),
            })
            .collect();
        resolve.insert(lookup.name.clone(), json!(answers));
    }
    let (allowed, blocked) = case.domains.as_ref().map_or_else(
        || (vec!["*".to_owned()], Vec::new()),
        |domains| (domains.allowed.clone(), domains.blocked.clone()),
    );
    let mut ranges = program.private_ranges.clone();
    ranges.extend(listed.iter().cloned());
    let mut settings = json!({
        "network": { "resolve": resolve },
        "egress": {
            "allowed_domains": allowed,
            "blocked_domains": blocked,
            "private_ranges": ranges,
        },
    });
    if case.route == Route::UpstreamProxy {
        settings["proxy"] = json!({
            "server": format!("http://{}", proving_ground.description().fixtures.proxies["open"]),
        });
    }
    settings
}

/// What the Proving ground's proxy `proxy` has seen so far: the connections
/// it accepted, the requests it received to pass on, and the `CONNECT`
/// tunnels it was asked for.
#[derive(Clone, Copy)]
struct Proxied {
    connections: usize,
    requests: usize,
    tunnels: usize,
}

impl Proxied {
    fn now(proving_ground: &ProvingGround, proxy: &str) -> Self {
        let observations = proving_ground.observations().unwrap();
        let methods: Vec<&Value> = observations["relays"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|relay| relay["proxy"] == json!(proxy))
            .map(|relay| &relay["method"])
            .collect();
        let tunnels = methods
            .iter()
            .filter(|method| **method == &json!("CONNECT"))
            .count();
        Self {
            connections: listener_connections(proving_ground, proxy),
            requests: methods.len() - tunnels,
            tunnels,
        }
    }

    /// How many times a navigation reached the proxy since `before`, on a
    /// Session leaving as `egress`, `refused` when the interception refused
    /// it. Only the Session's egress relay connects to the proxy, plain or
    /// `https://`, once it judged the case, so each connection counts, a
    /// TLS handshake that failed included. With no relay, the browser
    /// preconnects to its proxy as a navigation starts, before the
    /// interception judged it, as `borrowed_browser.rs` says of a site: a
    /// connection that carries nothing, or, for an `https://` case, its
    /// `CONNECT` tunnel, the gap ADR 0017's #2435 note names. A refused
    /// navigation's request is never sent, so there a request the proxy
    /// received counts.
    fn since(self, before: Self, egress: Egress, refused: bool) -> usize {
        match (egress, refused) {
            (Egress::Relay | Egress::HttpsProxy, _) => self.connections - before.connections,
            (Egress::SwitchedOff, false) => {
                self.requests + self.tunnels - before.requests - before.tunnels
            }
            (Egress::SwitchedOff, true) => self.requests - before.requests,
        }
    }
}

/// What one case's navigation did in the Run `run_id`, on a page of its
/// own, closed once it answered, so no refusal of a case's reaches
/// another's call: the refusal its `page.goto` raised, and a connection to
/// the Proving ground's address for each request it received meanwhile and
/// to the case's target for each time it reached the proxy behind it, which
/// would pass it on (see [`Proxied::since`]).
fn navigation(
    program: &mut Program,
    proving_ground: &ProvingGround,
    run_id: &str,
    case: &SsrfCase,
) -> Outcome {
    let public = proving_ground.address().ip();
    let page = program.new_page(run_id);
    let requested = requests(proving_ground).len();
    let proxy = program.egress.proxy();
    let proxied = Proxied::now(proving_ground, proxy);
    let went = program.goto(&page, &navigated_url(case, proving_ground.address().port()));
    let _ = program.call("page.close", &json!({ "page_id": page }));
    let reached = requests(proving_ground).len() - requested;
    let refusal = match error(&went) {
        (code, details) if code == "egress_refused" => {
            serde_json::from_value(details["rule"].clone())
                .ok()
                .map(|rule| Refusal::EgressRefused { rule })
        }
        _ => None,
    };
    let proxied_now =
        Proxied::now(proving_ground, proxy).since(proxied, program.egress, refusal.is_some());
    Outcome {
        refusal,
        connections: [
            vec![public; reached],
            vec![case.target_address(public).unwrap_or(public); proxied_now],
        ]
        .concat(),
    }
}

/// The SSRF table's thin runner on Chromium (ADR 0067) for the program's
/// own navigation, `test` the runner's test on `layer`, its Sessions
/// leaving as `egress` says: each of its cases as `page.goto`, at the
/// Proving ground's port, is refused with `egress_refused` and its rule,
/// and no request reaches the Proving ground meanwhile, directly or through
/// its proxy, which counts as [`Proxied::since`] says. One Run per domain
/// policy, proxy and mapping, which its cases
/// share, finished once they are done, so the test holds one browser and
/// one page at a time: the Resource governor admits no page past its
/// floor, one, while the host is under pressure (ADR 0040). The positive
/// controls, a Proving ground page directly and a public name through the
/// open proxy, reach the Proving ground. Chromium and the engine read a URL
/// as the WHATWG parser does, so each URL is navigated as that parser
/// writes it, but for those it refuses, navigated as written. Each Run's
/// Provenance records the rebinding gap when no relay holds its Session.
/// Behind the `https://` proxy, every case goes through it, the relay
/// trusting its authority as the user configuration says.
fn navigations_are_refused_on_chromium(layer: &Layer, test: &str, egress: Egress) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    if egress == Egress::HttpsProxy {
        trusting_the_tls_proxy(&env, &proving_ground);
    }
    let cache = browser_cache(&env).unwrap();
    let public = proving_ground.address().ip();
    let runner = Runner::committed(layer.name, file!(), test).unwrap();
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "public_page"
        family = "mixed_answers"
        url = "http://catalog.test/catalog/1"
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }

        [[cases]]
        name = "public_name_through_the_proxy"
        family = "upstream_proxy"
        url = "http://proxied.ssrf.test/"
        route = { via = "upstream_proxy" }
        lookups = [{ name = "proxied.ssrf.test", answers = ["public"] }]
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();
    let listed = ["proxied.ssrf.test".to_owned()];
    let mut program =
        Program::start(&env, &cache, &proving_ground, Surface::SdkPython).leaving(egress);
    let mut groups: Vec<(Value, Vec<&SsrfCase>)> = Vec::new();
    for case in controls.cases.iter().chain(runner.cases()) {
        let mut settings = case_settings(&program, &proving_ground, case, &listed);
        egress.apply(&mut settings, &program.tls_proxy);
        match groups.iter_mut().find(|(shared, _)| *shared == settings) {
            Some((_, members)) => members.push(case),
            None => groups.push((settings, vec![case])),
        }
    }
    let mut outcomes: BTreeMap<String, Outcome> = BTreeMap::new();
    let mut run_ids = Vec::new();
    for (index, (settings, members)) in groups.into_iter().enumerate() {
        let run_id = format!("run-{index}");
        let started = program.call(
            "run.start",
            &json!({ "run_id": run_id, "settings": settings }),
        );
        assert!(started.get("result").is_some(), "{started}");
        for case in members {
            let outcome = navigation(&mut program, &proving_ground, &run_id, case);
            outcomes.insert(case.name.clone(), outcome);
        }
        let _ = program.call("run.finish", &json!({ "run_id": run_id }));
        run_ids.push(run_id);
    }
    program.finish(&[]);

    let result = runner.run(&controls.cases, public, |case| {
        outcomes.remove(&case.name).unwrap_or_default()
    });
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
    assert_gap_recorded(&env, &run_ids, layer);
}

/// The address cases, every encoding, IPv6 form, special-purpose block,
/// mixed answer and upstream proxy's, on a launched Session with no egress
/// relay, which the interception alone holds: see
/// [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused, operation:page.goto
#[rstest]
/// Covers: setting:security.raw_cdp_egress_proxy
#[case::with_the_switch_off(Egress::SwitchedOff)]
fn every_address_case_of_the_ssrf_table_is_refused_at_interception_on_a_launched_session_with_no_relay(
    #[case] egress: Egress,
) {
    navigations_are_refused_on_chromium(
        &WITHOUT_RELAY,
        "every_address_case_of_the_ssrf_table_is_refused_at_interception_on_a_launched_session_with_no_relay",
        egress,
    );
}

/// The encodings' and the IPv6 forms' cases as navigations: see
/// [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused, operation:page.goto
#[test]
fn every_encoding_and_ipv6_form_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium() {
    navigations_are_refused_on_chromium(
        &RELAYED,
        "every_encoding_and_ipv6_form_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium",
        Egress::Relay,
    );
}

/// Every IANA block's case as a navigation: see
/// [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused, operation:page.goto
#[test]
fn every_special_purpose_block_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium() {
    navigations_are_refused_on_chromium(
        &RELAYED,
        "every_special_purpose_block_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium",
        Egress::Relay,
    );
}

/// The mixed answers', upstream proxy's and domain policy's cases as
/// navigations: see [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-2, story:4-10, story:4-12, story:4-89, error:egress_refused, operation:page.goto
#[test]
fn every_name_and_proxy_case_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium() {
    navigations_are_refused_on_chromium(
        &RELAYED,
        "every_name_and_proxy_case_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium",
        Egress::Relay,
    );
}

/// The same behind an `https://` proxy, which the egress relay chains
/// through inside TLS (issue 1609): a private answer is refused at the
/// relay before the proxy sees a connection. See
/// [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-10, story:4-12, setting:proxy.server, error:egress_refused, operation:page.goto
#[test]
fn every_name_and_proxy_case_of_the_ssrf_table_is_refused_as_a_navigation_behind_an_https_proxy_on_chromium()
 {
    navigations_are_refused_on_chromium(
        &RELAYED,
        "every_name_and_proxy_case_of_the_ssrf_table_is_refused_as_a_navigation_behind_an_https_proxy_on_chromium",
        Egress::HttpsProxy,
    );
}

/// A file of the machine the engine runs on, holding `text`.
fn local_file(env: &TestEnv, text: &str) -> String {
    let file = env.folder().join("report.html");
    std::fs::write(
        &file,
        format!("<!doctype html><html><body><main><h1>{text}</h1></main></body></html>"),
    )
    .unwrap();
    let path = file.display().to_string().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

const REPORT: &str = "The local report";

/// Covers: story:4-6, operation:page.goto
#[test]
fn the_programs_own_code_loads_a_file_url_on_a_local_engine() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let file = local_file(&env, REPORT);
    let mut program = Program::start(&env, &cache, &proving_ground, Surface::SdkPython);
    program.run("files", &["*"]);
    let page = program.new_page("files");

    let went = program.goto(&page, &file);
    let markdown = program.markdown(&page);
    program.finish(&["files"]);

    assert_eq!(
        (
            went.get("error"),
            markdown.map(|markdown| markdown.contains(REPORT))
        ),
        (None, Ok(true)),
        "{went}"
    );
}

/// Covers: story:4-6, error:egress_refused, operation:page.goto
#[test]
fn a_client_acting_for_a_model_is_refused_a_file_url() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let file = local_file(&env, REPORT);
    let mut program = Program::start(&env, &cache, &proving_ground, Surface::Mcp);
    program.run("files", &["*"]);
    let page = program.new_page("files");

    let went = program.goto(&page, &file);
    let markdown = program.markdown(&page);
    program.finish(&["files"]);

    assert_eq!(
        (
            error(&went),
            markdown.map(|markdown| markdown.contains(REPORT))
        ),
        (
            (
                json!("egress_refused"),
                json!({ "url": file, "rule": "scheme" })
            ),
            Ok(false)
        )
    );
}

/// A navigation the policy refuses answers once the browser's error page
/// for it has loaded, as a navigation to a document answers once that
/// document has: the next call finds the error page whole, never the page
/// before it nor one the browser is still loading.
///
/// Covers: story:4-5, error:egress_refused, operation:page.goto, operation:page.formats
#[test]
fn a_refused_navigation_answers_once_the_browsers_error_page_for_it_loaded() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground, Surface::SdkPython);
    program.run("refused", &["catalog.test"]);
    let page = program.new_page("refused");

    let went = program.goto(&page, &proving_ground.url("elsewhere.test", "/offsite"));
    let read = program.call(
        "page.formats",
        &json!({ "page_id": page, "formats": { "markdown": true } }),
    );
    program.finish(&["refused"]);

    assert_eq!(
        (error(&went).0, &read["result"]["url"]),
        (
            json!("egress_refused"),
            &json!("chrome-error://chromewebdata/")
        ),
        "{read}"
    );
}

/// The `javascript:` URL the tests run: it writes the page's main content.
const SCRIPT: &str =
    "javascript:void(document.body.innerHTML='<main><h1>Written by the script</h1></main>')";

/// A `javascript:` URL in each spelling the URL Standard reads as one, a
/// tab or a newline in its scheme or spaces before it, runs as the plain
/// form does and answers as it does: the driver tells it apart as the
/// egress check does (the deep review's 563-4).
///
/// Covers: story:4-5, operation:page.goto
#[rstest]
#[case::plain(SCRIPT.to_owned())]
#[case::a_tab_in_its_scheme(SCRIPT.replacen("java", "java\t", 1))]
#[case::a_newline_in_its_scheme(SCRIPT.replacen("java", "java\n", 1))]
#[case::spaces_before_it(format!("  {SCRIPT}"))]
fn a_javascript_url_runs_in_a_frame_the_policy_allows_and_opens_no_connection(
    #[case] script: String,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground, Surface::SdkPython);
    program.run("scripts", &["catalog.test"]);
    let page = program.new_page("scripts");
    let went = program.goto(&page, &proving_ground.url("catalog.test", "/catalog/1"));
    assert!(went.get("result").is_some(), "{went}");
    // The document's robots.txt read, which its `goto` never waits for,
    // has sent its request once the Run recorded its reading: no request
    // the document's load started comes after the count.
    robots_txt_read(&env, "scripts");
    let before = pages_requested(&proving_ground);

    let ran = program.goto(&page, &script);
    let markdown = program.markdown(&page);
    let after = pages_requested(&proving_ground);
    program.finish(&["scripts"]);

    assert_eq!(
        (
            &ran["result"],
            markdown.map(|markdown| markdown.contains("Written by the script")),
            after - before,
        ),
        (&json!({ "response": null }), Ok(true), 0),
        "{ran}"
    );
}

/// Covers: story:4-5, error:egress_refused, operation:page.goto
#[test]
fn a_javascript_url_in_a_frame_whose_origin_the_policy_refuses_does_not_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground, Surface::SdkPython);
    program.run("scripts", &["catalog.test"]);
    let page = program.new_page("scripts");
    let went = program.goto(&page, &proving_ground.url("elsewhere.test", "/offsite"));
    assert_eq!(error(&went).0, json!("egress_refused"), "{went}");

    let ran = program.goto(&page, SCRIPT);
    let markdown = program.markdown(&page);
    program.finish(&["scripts"]);

    assert_eq!(
        (
            error(&ran).0,
            error(&ran).1["url"].clone(),
            markdown.map(|markdown| markdown.contains("Written by the script")),
        ),
        (json!("egress_refused"), json!(SCRIPT), Ok(false))
    );
}

/// Rebinding end to end on Chromium (ADR 0067, ticket 1628): Collie's
/// executable and Chromium run in a `DnsNamespace` whose resolver is the
/// Proving ground's DNS fixture. Each rebinding case of the SSRF table, as
/// a navigation, reaches its public stand-in once and is refused
/// (`private_address`) on its next connection, with no connection reaching
/// the target.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_types,
    reason = "the test's servers bind their own sockets on loopback, as sites outside the engine do (ADR 0060 covers the engine's sockets)"
)]
mod linux {
    use std::io::{ErrorKind, Read as _, Write as _};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use goodfetch_protocol::catalog::Product;
    use goodfetch_protocol::connection::Surface;
    use goodfetch_test_client::{Attached, Hello};
    use goodfetch_test_support::{
        Answer, BROWSERS_PATH_VARIABLE, DnsNamespace, Outcome, ProvingGround, Refusal, Runner,
        SsrfCase, SsrfTable, TestEnv, browser_cache,
    };
    use serde_json::{Value, json};
    use url::Url;

    use super::{COLLIE, RELAYED, SEED, error, opened};
    use crate::page::BROWSER_DEADLINE;

    const CHILD: &str = "navigation_egress::linux::in_the_namespace_every_rebinding_case_is_refused_as_a_navigation_on_chromium";

    /// The `alternating.dns.test` label for an IP address.
    fn dns_label(address: IpAddr) -> String {
        match address {
            IpAddr::V4(v4) => v4.octets().map(|octet| octet.to_string()).join("-"),
            IpAddr::V6(v6) => format!("{:032x}", u128::from(v6)),
        }
    }

    /// An `alternating.dns.test` host name that answers `public` on its first
    /// lookup and `rebound` on its next two lookups (repeating `rebound` so a
    /// background lookup after the first navigation cannot wrap the fixture
    /// back to `public` before the second navigation). When `rebound` is IPv6,
    /// `nx` separates the two turns so the first lookup yields only `public`
    /// and the next yield only `rebound`.
    fn alternating_name(public: Ipv4Addr, rebound: IpAddr) -> String {
        let first = dns_label(IpAddr::V4(public));
        let second = dns_label(rebound);
        match rebound {
            IpAddr::V4(_) => format!("{first}.{second}.{second}.alternating.dns.test"),
            IpAddr::V6(_) => format!("{first}.nx.nx.{second}.{second}.alternating.dns.test"),
        }
    }

    /// A loopback HTTP server answering `200 OK` with `Connection: close` so
    /// each navigation opens a fresh TCP connection through the egress relay.
    struct PublicStandIn {
        port: u16,
        accepted: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        join: Option<thread::JoinHandle<()>>,
    }

    impl PublicStandIn {
        fn start(address: Ipv4Addr) -> Self {
            let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(address), 0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let accepted = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let accepted_in_thread = Arc::clone(&accepted);
            let stop_in_thread = Arc::clone(&stop);
            let join = thread::spawn(move || {
                while !stop_in_thread.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            accepted_in_thread.fetch_add(1, Ordering::SeqCst);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let mut buf = [0u8; 4096];
                            let mut read = 0;
                            while read < buf.len() {
                                match stream.read(&mut buf[read..]) {
                                    Ok(0) | Err(_) => break,
                                    Ok(n) => {
                                        read += n;
                                        if buf[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                                            break;
                                        }
                                    }
                                }
                            }
                            let body = b"<html><head><link rel=\"icon\" href=\"data:,\"></head><body>ok</body></html>";
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.write_all(body);
                            let _ = stream.flush();
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                port,
                accepted,
                stop,
                join: Some(join),
            }
        }

        fn connections(&self) -> usize {
            self.accepted.load(Ordering::SeqCst)
        }
    }

    impl Drop for PublicStandIn {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    fn run_rebinding_case(
        engine: &mut Attached,
        next_id: &mut i64,
        run_id: &str,
        case: &SsrfCase,
        public: Ipv4Addr,
    ) -> Outcome {
        let target = case.target_address(IpAddr::V4(public)).unwrap();
        let server = PublicStandIn::start(public);
        let port = server.port;
        let watched = (target != IpAddr::V4(public) && target.is_loopback()).then(|| {
            let listener = TcpListener::bind(SocketAddr::new(target, port)).unwrap();
            listener.set_nonblocking(true).unwrap();
            listener
        });
        let host = alternating_name(public, target);
        let parsed = Url::parse(&case.url).unwrap();
        let path = parsed.path();
        let url = format!("http://{host}:{port}{path}");
        let settings = json!({
            "egress": {
                "allowed_domains": [host],
                "private_ranges": [public.to_string()],
            },
        });
        let mut ask = |method: &str, params: &Value| {
            *next_id += 1;
            engine.ask(*next_id, method, params)
        };
        let started = ask(
            "run.start",
            &json!({ "run_id": run_id, "settings": settings }),
        );
        assert!(started.get("result").is_some(), "{started}");
        let page = opened(&ask("run.new_page", &json!({ "run_id": run_id })))
            .page_id
            .0;

        let first = ask(
            "page.goto",
            &json!({ "page_id": page, "url": format!("{url}?n=1") }),
        );
        assert_eq!(
            first["result"]["response"]["status"],
            json!(200),
            "case {}: first navigation reaches its public stand-in: {first}",
            case.name
        );
        assert!(
            server.connections() >= 1,
            "case {}: public stand-in accepted no connection on first navigation",
            case.name
        );

        let second = ask(
            "page.goto",
            &json!({ "page_id": page, "url": format!("{url}?n=2") }),
        );
        let _ = ask("run.finish", &json!({ "run_id": run_id }));

        let mut connections = Vec::new();
        if target == IpAddr::V4(public) && server.connections() >= 2 {
            connections.push(IpAddr::V4(public));
        }
        if let Some(watched) = watched {
            match watched.accept() {
                Ok(_) => connections.push(target),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => panic!("watched target listener failed: {error}"),
            }
        }
        let refusal = match error(&second) {
            (code, details) if code == "egress_refused" => {
                serde_json::from_value(details["rule"].clone())
                    .ok()
                    .map(|rule| Refusal::EgressRefused { rule })
            }
            _ => None,
        };
        Outcome {
            refusal,
            connections,
        }
    }

    /// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused, operation:page.goto
    #[test]
    fn every_rebinding_case_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium() {
        let env = TestEnv::new();
        let cache = browser_cache(&env).unwrap();
        let env = env.with_variable(BROWSERS_PATH_VARIABLE, cache.to_str().unwrap());
        let proving_ground = ProvingGround::start(&env, SEED).unwrap();
        let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
        let namespace = DnsNamespace::enter(&env, fixture).unwrap();

        let output = namespace
            .output(
                &env,
                std::env::current_exe().unwrap(),
                [CHILD, "--include-ignored"],
            )
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && !stdout.contains("running 0 tests"),
            "status={:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
    }

    #[ignore = "run as a child by navigation_egress::linux::every_rebinding_case_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium"]
    #[test]
    fn in_the_namespace_every_rebinding_case_is_refused_as_a_navigation_on_chromium() {
        let env = TestEnv::new();
        let cache = std::path::PathBuf::from(std::env::var(BROWSERS_PATH_VARIABLE).unwrap());
        let runner = Runner::committed(
            RELAYED.name,
            file!(),
            "every_rebinding_case_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium",
        )
        .unwrap();
        let controls = SsrfTable::parse(
            r#"
            [[cases]]
            name = "public_rebinding_control"
            family = "rebinding"
            url = "http://rebind.ssrf.test/"
            fetches = 2
            target = "public"
            refusal = { code = "egress_refused", rule = "private_address" }
            "#,
        )
        .unwrap();

        let mut engine = Attached::start_with(&env, COLLIE, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, &cache);
        });
        engine.send(
            &Hello::needing(Product::Collie)
                .surface(Surface::SdkPython)
                .message(),
        );
        let _ = engine.next();
        let mut next_id = 0;

        let public = Ipv4Addr::new(127, 0, 0, 10);
        let mut index = 0usize;
        let result = runner.run(&controls.cases, IpAddr::V4(public), |case| {
            index += 1;
            let stand_in = Ipv4Addr::new(127, 0, 0, 10 + u8::try_from(index).unwrap());
            let run_id = format!("rebind-{index}");
            // Note: Runner checks target_address(public) for controls, so
            // controls use `public` itself while each rebinding case gets its
            // own stand-in address to keep alternating DNS turns isolated.
            let address = if case.target == Some(Answer::Public) {
                public
            } else {
                stand_in
            };
            run_rebinding_case(&mut engine, &mut next_id, &run_id, case, address)
        });
        let _ = engine.finish();

        assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
    }
}
