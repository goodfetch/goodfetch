//! A page's navigations under the egress policy over stdio (spec #563's
//! stories 5 and 6; ADRs 0017, 0056, 0089 and 0120), against the Proving
//! ground's redirect pages, whose connection record is the oracle: every
//! hop of a redirect of any status or kind is judged again, one to a
//! scheme other than HTTP's is refused with `egress_refused`, a `file:`
//! URL is loaded only for the program's own code, and a `javascript:` URL
//! runs only in a frame whose origin the domain policy allows.
//!
//! The browser layer's resolved-address check is the egress relay's
//! (#634), so the SSRF table's redirects to a private address run here
//! under a domain policy that refuses their target instead: each status
//! and kind is still re-checked at its hop.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests poll the Proving ground and the page on their own thread, never an async worker"
)]

use std::net::IpAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, CaseDomains, EgressRule, Family, Outcome, ProvingGround, Refusal,
    SsrfCase, SsrfTable, TestEnv, browser_cache, run_cases,
};
use serde_json::{Map, Value, json};

use crate::attached::{Attached, hello};
use crate::page::{BROWSER_DEADLINE, opened};
use crate::run::call;

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

/// A Connection to an Attached engine whose `hello` names `surface`, and
/// the Proving ground's hosts mapped in each Run's `network.resolve`.
struct Program<'env> {
    engine: Attached<'env>,
    next_id: i64,
    resolve: Map<String, Value>,
}

impl<'env> Program<'env> {
    fn start(
        env: &'env TestEnv,
        cache: &Path,
        proving_ground: &ProvingGround,
        surface: &str,
    ) -> Self {
        let mut engine = Attached::start_waiting(env, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
        });
        let mut greeting = hello("collie", "0.1");
        greeting["params"]["surface"] = json!(surface);
        engine.send(&greeting);
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
            resolve,
        }
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        call(&mut self.engine, self.next_id, method, params)
    }

    /// Starts the Run `run_id` whose domain policy allows `allowed`.
    fn run(&mut self, run_id: &str, allowed: &[&str]) {
        let settings = json!({
            "network": { "resolve": self.resolve },
            "egress": { "allowed_domains": allowed },
        });
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

/// The requests the Proving ground received, as `(host, target)`.
fn requests(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|request| {
            (
                request["host"].as_str().unwrap_or_default().to_owned(),
                request["target"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// How many requests for a page, not the browser's own for an icon, the
/// Proving ground received.
fn pages_requested(proving_ground: &ProvingGround) -> usize {
    requests(proving_ground)
        .iter()
        .filter(|(_, target)| target != "/favicon.ico")
        .count()
}

/// Covers: story:4-5, error:egress_refused, operation:page.goto
#[test]
fn a_chain_whose_third_hop_leaves_the_policy_is_refused_at_that_hop() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground, "sdk_python");
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

    // The browser's own request for the page's icon may come before the
    // `goto` answers, so only the page's own requests count.
    let reached = |requests: &[(String, String)]| {
        requests
            .iter()
            .filter(|(host, target)| host == "catalog.test" && target != "/favicon.ico")
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
    program: &mut Program<'_>,
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
            .filter(|(host, target)| {
                !(*host == own.0 && *target == own.1) && target != "/favicon.ico"
            })
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

/// The SSRF table's thin runner on Chromium (ADR 0067), for the redirect
/// family: each case's page as the Proving ground serves it, every status
/// and kind of redirect and every scheme other than HTTP's. A case aiming
/// at a private address runs under a domain policy of the stand-ins' host,
/// so its hop is refused by the policy, until the relay (#634) judges the
/// address itself. Positive controls: the same kinds to an allowed page go
/// through.
///
/// Covers: story:4-5, error:egress_refused
#[test]
fn every_redirect_case_of_the_ssrf_table_is_refused_at_its_hop_on_chromium() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let public = proving_ground.address().ip();
    let table = SsrfTable::committed().unwrap();
    let cases: Vec<SsrfCase> = table
        .cases_of(Family::Redirect)
        .map(|case| match &case.refusal {
            Refusal::EgressRefused {
                rule: EgressRule::PrivateAddress,
            } => SsrfCase {
                domains: Some(CaseDomains {
                    allowed: vec!["redirects.test".to_owned()],
                    blocked: Vec::new(),
                }),
                refusal: Refusal::EgressRefused {
                    rule: EgressRule::DomainPolicy,
                },
                ..case.clone()
            },
            _ => case.clone(),
        })
        .collect();
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
    let mut program = Program::start(&env, &cache, &proving_ground, "sdk_python");
    // One Run per domain policy, which its pages share.
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut run_for = |program: &mut Program<'_>, case: &SsrfCase| -> String {
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

    let controlled = run_cases(&controls.cases, public, |case| {
        let run_id = run_for(&mut program, case);
        run_case(&mut program, &proving_ground, &run_id, case)
    });
    let result = run_cases(&cases, public, |case| {
        let run_id = run_for(&mut program, case);
        run_case(&mut program, &proving_ground, &run_id, case)
    });
    let finished: Vec<String> = (0..runs.len()).map(|run| format!("policy-{run}")).collect();
    let finished: Vec<&str> = finished.iter().map(String::as_str).collect();
    program.finish(&finished);

    assert_eq!(
        controlled.map_err(|let_through| {
            let_through
                .misses
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect::<Vec<_>>()
        }),
        Err(vec![
            ("status_to_an_allowed_host".to_owned(), None, true),
            ("meta_refresh_to_an_allowed_page".to_owned(), None, true),
            ("script_to_an_allowed_page".to_owned(), None, true),
        ])
    );
    assert_eq!(cases.len(), 15);
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
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
    let mut program = Program::start(&env, &cache, &proving_ground, "sdk_python");
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
    let mut program = Program::start(&env, &cache, &proving_ground, "mcp");
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
    let mut program = Program::start(&env, &cache, &proving_ground, "sdk_python");
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

/// Covers: story:4-5, operation:page.goto
#[test]
fn a_javascript_url_runs_in_a_frame_the_policy_allows_and_opens_no_connection() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground, "sdk_python");
    program.run("scripts", &["catalog.test"]);
    let page = program.new_page("scripts");
    let went = program.goto(&page, &proving_ground.url("catalog.test", "/catalog/1"));
    assert!(went.get("result").is_some(), "{went}");
    let before = pages_requested(&proving_ground);

    let ran = program.goto(&page, SCRIPT);
    let markdown = program.markdown(&page);
    let after = pages_requested(&proving_ground);
    program.finish(&["scripts"]);

    assert_eq!(
        (
            &ran["result"],
            markdown.map(|markdown| markdown.contains("Written by the script")),
            after - before,
        ),
        (&json!({ "response": null }), Ok(true), 0)
    );
}

/// Covers: story:4-5, error:egress_refused, operation:page.goto
#[test]
fn a_javascript_url_in_a_frame_whose_origin_the_policy_refuses_does_not_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &proving_ground, "sdk_python");
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
