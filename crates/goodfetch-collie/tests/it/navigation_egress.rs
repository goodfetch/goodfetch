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
//! where their hop connects, and raise `egress_refused` naming the address.
//! A launched Session with no relay under it, with
//! `security.raw_cdp_egress_proxy` off or behind an `https://` proxy, is
//! held by interception alone, which resolves each host in the engine and
//! judges every answer (#2435).
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
    /// Through an `https://` proxy, which the relay does not chain: the
    /// Proving ground's open proxy, which speaks no TLS, so a request sent
    /// on stops at its handshake, and the proxy's connections show it was.
    HttpsProxy,
}

impl Egress {
    /// Sets this egress in `settings`, a Run's, `open_proxy` the Proving
    /// ground's open proxy's address.
    fn apply(self, settings: &mut Value, open_proxy: &str) {
        match self {
            Self::Relay => {}
            Self::SwitchedOff => {
                settings["security"] = json!({ "raw_cdp_egress_proxy": false });
            }
            Self::HttpsProxy => {
                settings["proxy"] = json!({ "server": format!("https://{open_proxy}") });
            }
        }
    }
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
    open_proxy: String,
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
            open_proxy: proving_ground.description().fixtures.proxies["open"].clone(),
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
        self.egress.apply(&mut settings, &self.open_proxy);
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

    assert_eq!(cases.len(), 15);
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

/// What the Proving ground's open proxy has seen so far: the connections it
/// accepted, the requests it received to pass on, and the `CONNECT`
/// tunnels it was asked for.
#[derive(Clone, Copy)]
struct Proxied {
    connections: usize,
    requests: usize,
    tunnels: usize,
}

impl Proxied {
    fn now(proving_ground: &ProvingGround) -> Self {
        let observations = proving_ground.observations().unwrap();
        let methods: Vec<&Value> = observations["relays"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|relay| relay["proxy"] == json!("open"))
            .map(|relay| &relay["method"])
            .collect();
        let tunnels = methods
            .iter()
            .filter(|method| **method == &json!("CONNECT"))
            .count();
        Self {
            connections: listener_connections(proving_ground, "open"),
            requests: methods.len() - tunnels,
            tunnels,
        }
    }

    /// How many times a navigation reached the proxy since `before`, on a
    /// Session leaving as `egress`, `refused` when the interception refused
    /// it. Only the Session's egress relay connects to the proxy, once it
    /// judged the case, so each connection counts. With no relay, the
    /// browser preconnects to its proxy as a navigation starts, before the
    /// interception judged it, as `borrowed_browser.rs` says of a site: a
    /// connection that carries nothing, or, for an `https://` case through
    /// the plain proxy, its `CONNECT` tunnel, the gap ADR 0017's #2435 note
    /// names. A refused navigation's request is never sent, so there a
    /// request the plain proxy received counts, and nothing behind the
    /// `https://` proxy, which speaks no TLS and reads none.
    fn since(self, before: Self, egress: Egress, refused: bool) -> usize {
        match (egress, refused) {
            (Egress::Relay, _) | (Egress::HttpsProxy, false) => {
                self.connections - before.connections
            }
            (Egress::HttpsProxy, true) => 0,
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
/// to the case's target for each time it reached the open proxy, which
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
    let proxied = Proxied::now(proving_ground);
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
        Proxied::now(proving_ground).since(proxied, program.egress, refusal.is_some());
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
/// its open proxy, which counts as [`Proxied::since`] says. One Run per domain policy, proxy and mapping, which its cases
/// share, finished once they are done, so the test holds one browser and
/// one page at a time: the Resource governor admits no page past its
/// floor, one, while the host is under pressure (ADR 0040). The positive
/// controls, a Proving ground page directly and a public name through the
/// open proxy, reach the Proving ground. Chromium and the engine read a URL
/// as the WHATWG parser does, so each URL is navigated as that parser
/// writes it, but for those it refuses, navigated as written. Each Run's
/// Provenance records the rebinding gap when no relay holds its Session.
fn navigations_are_refused_on_chromium(layer: &Layer, test: &str, egress: Egress) -> Runner {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
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
        egress.apply(&mut settings, &program.open_proxy);
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
    runner
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
/// Covers: setting:proxy.server
#[case::behind_an_https_proxy(Egress::HttpsProxy)]
fn every_address_case_of_the_ssrf_table_is_refused_at_interception_on_a_launched_session_with_no_relay(
    #[case] egress: Egress,
) {
    let runner = navigations_are_refused_on_chromium(
        &WITHOUT_RELAY,
        "every_address_case_of_the_ssrf_table_is_refused_at_interception_on_a_launched_session_with_no_relay",
        egress,
    );

    assert!(
        runner.cases().len() >= 70,
        "only {} cases",
        runner.cases().len()
    );
}

/// The encodings' and the IPv6 forms' cases as navigations: see
/// [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused, operation:page.goto
#[test]
fn every_encoding_and_ipv6_form_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium() {
    let runner = navigations_are_refused_on_chromium(
        &RELAYED,
        "every_encoding_and_ipv6_form_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium",
        Egress::Relay,
    );

    assert!(
        runner.cases().len() >= 30,
        "only {} cases",
        runner.cases().len()
    );
}

/// Every IANA block's case as a navigation: see
/// [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-5, story:4-10, story:4-89, error:egress_refused, operation:page.goto
#[test]
fn every_special_purpose_block_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium() {
    let runner = navigations_are_refused_on_chromium(
        &RELAYED,
        "every_special_purpose_block_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium",
        Egress::Relay,
    );

    assert!(
        runner.cases().len() >= 36,
        "only {} cases",
        runner.cases().len()
    );
}

/// The mixed answers', upstream proxy's and domain policy's cases as
/// navigations: see [`navigations_are_refused_on_chromium`].
///
/// Covers: story:4-2, story:4-10, story:4-12, story:4-89, error:egress_refused, operation:page.goto
#[test]
fn every_name_and_proxy_case_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium() {
    let runner = navigations_are_refused_on_chromium(
        &RELAYED,
        "every_name_and_proxy_case_of_the_ssrf_table_is_refused_as_a_navigation_on_chromium",
        Egress::Relay,
    );

    assert!(
        runner.cases().len() >= 25,
        "only {} cases",
        runner.cases().len()
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
