//! The service-worker Challenge (#1539, ADR 0035): its page shows its article
//! only to a client whose request a registered service worker answered, and
//! the observations say whether one did.
//!
//! Each test plays a browser's requests, the worker's included, with a plain
//! HTTP client.
//! Covers: story:2-33

use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{Answer, ProvingGround, submission};

const SEED: u64 = 1539;

const TASK: &str = "protection/5";
const HOST: &str = "5.protection.test";
const LOCAL_HOST: &str = "protection.localhost";

const PAGE: &str = "/service-worker";
const WORKER: &str = "/service-worker.js";
const CHECK: &str = "/service-worker/check";

/// The header a browser sends as it fetches a service worker's script, and
/// no page's script or resource.
const WORKER_REQUEST: (&str, &str) = ("service-worker", "script");

/// The text of `body` between `before` and the next `after`.
fn between<'a>(body: &'a str, before: &str, after: &str) -> &'a str {
    body.split_once(before)
        .and_then(|(_, rest)| rest.split_once(after))
        .unwrap_or_else(|| panic!("{before} in {body}"))
        .0
}

/// The worker's script, fetched as a browser registering it does.
fn worker(proving_ground: &ProvingGround, host: &str) -> Answer {
    proving_ground.get_with_headers(&proving_ground.url(host, WORKER), &[WORKER_REQUEST])
}

/// The proof the worker's script sends.
fn proof(proving_ground: &ProvingGround, host: &str) -> String {
    let script = worker(proving_ground, host);
    assert_eq!(script.status, 200, "{}", script.body);
    between(&script.body, "const PROOF = \"", "\"").to_owned()
}

/// Posts the page's check, as the worker that answers it does when `proof`
/// is one and as the page itself does otherwise.
fn check(proving_ground: &ProvingGround, host: &str, proof: Option<&str>) -> Answer {
    let headers = proof.map(|proof| ("x-service-worker", proof));
    proving_ground.post_json_with_headers(
        &proving_ground.url(host, CHECK),
        headers.as_slice(),
        &json!({}),
    )
}

/// What the observations say of the service-worker Challenge: the task and
/// the observed value of each report.
fn observed(proving_ground: &ProvingGround) -> Vec<(Value, Value)> {
    proving_ground.observations()["reports"]
        .as_array()
        .expect("`reports` is an array")
        .iter()
        .map(|report| {
            assert_eq!(report["page"], PAGE);
            (report["task"].clone(), report["observed"].clone())
        })
        .collect()
}

#[test]
fn the_task_starts_in_a_secure_context() {
    let proving_ground = ProvingGround::start(SEED);
    let task = proving_ground.task(TASK);
    assert_eq!(
        task["start_url"],
        format!("https://{HOST}:{}{PAGE}", proving_ground.port())
    );
    assert_eq!(task["kind"], "extract");
}

#[rstest]
#[case::its_own_host(HOST, json!(TASK))]
#[case::a_secure_context_over_plain_http(LOCAL_HOST, Value::Null)]
fn the_page_passes_with_a_service_worker(#[case] host: &str, #[case] task: Value) {
    let proving_ground = ProvingGround::start(SEED);
    let url = proving_ground.url(host, PAGE);
    let challenge = proving_ground.get(&url);
    assert_eq!(challenge.status, 403, "{}", challenge.body);
    assert!(
        challenge.body.contains(&format!("register(\"{WORKER}\"")),
        "{}",
        challenge.body
    );
    let proof = proof(&proving_ground, host);

    let passed = check(&proving_ground, host, Some(&proof));
    assert_eq!(passed.status, 204, "{}", passed.body);
    let cookie = passed.cookie("service_worker").expect("a clearance");
    let article = proving_ground.get_with_cookie(&url, Some(&cookie));

    assert_eq!(article.status, 200, "{}", article.body);
    assert_eq!(observed(&proving_ground), [(task, json!("service_worker"))]);
    let record = json!({
        "id": between(&article.body, "data-id=\"", "\""),
        "title": between(&article.body, "<h1 class=\"title\">", "</h1>"),
    });
    let answer = proving_ground.judge(&submission(TASK, "success", &[record]));
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn the_page_fails_without_a_service_worker() {
    let proving_ground = ProvingGround::start(SEED);

    let refused = check(&proving_ground, HOST, None);

    assert_eq!(refused.status, 403, "{}", refused.body);
    assert_eq!(refused.cookie("service_worker"), None);
    assert_eq!(
        observed(&proving_ground),
        [(json!(TASK), json!("no_service_worker"))]
    );
    let page = proving_ground.get(&proving_ground.url(HOST, PAGE));
    assert_eq!(page.status, 403, "{}", page.body);
    assert!(!page.body.contains("data-id"), "{}", page.body);
}

#[rstest]
#[case::no_proof_at_all("")]
#[case::another_proof("0000000000000000")]
fn a_proof_the_worker_does_not_hold_is_refused(#[case] proof: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let refused = check(&proving_ground, HOST, Some(proof));

    assert_eq!(refused.status, 403, "{}", refused.body);
    assert_eq!(
        observed(&proving_ground),
        [(json!(TASK), json!("no_service_worker"))]
    );
}

#[rstest]
#[case::no_cookie(None)]
#[case::another_value(Some("service_worker=0000000000000000"))]
#[case::the_proof(Some("proof"))]
fn the_page_takes_only_the_clearance_it_gave(#[case] cookie: Option<&str>) {
    let proving_ground = ProvingGround::start(SEED);
    let proof = format!("service_worker={}", proof(&proving_ground, HOST));
    let cookie = cookie.map(|cookie| if cookie == "proof" { &proof } else { cookie });

    let page = proving_ground.get_with_cookie(&proving_ground.url(HOST, PAGE), cookie);

    assert_eq!(page.status, 403, "{}", page.body);
}

#[test]
fn a_page_can_not_read_the_worker_as_a_script() {
    let proving_ground = ProvingGround::start(SEED);

    let script = proving_ground.get(&proving_ground.url(HOST, WORKER));

    assert_eq!(script.status, 403, "{}", script.body);
    assert!(!script.body.contains("PROOF"), "{}", script.body);
}

#[test]
fn the_worker_is_a_script_a_browser_registers() {
    let proving_ground = ProvingGround::start(SEED);

    let script = worker(&proving_ground, HOST);

    assert_eq!(
        script.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert!(script.body.contains(CHECK), "{}", script.body);
}

#[test]
fn a_seed_gives_its_own_proof_and_article() {
    let first = ProvingGround::start(SEED);
    let again = ProvingGround::start(SEED);
    let other = ProvingGround::start(SEED + 1);

    assert_eq!(proof(&first, HOST), proof(&again, HOST));
    assert_ne!(proof(&first, HOST), proof(&other, HOST));
}

#[test]
fn a_guessed_record_is_not_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&submission(
        TASK,
        "success",
        &[json!({ "id": "T-0009", "title": "The Unseen Article" })],
    ));

    assert_ne!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_refused_check_leaves_a_later_pass_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let url = proving_ground.url(HOST, PAGE);
    check(&proving_ground, HOST, None);
    let proof = proof(&proving_ground, HOST);
    let cookie = check(&proving_ground, HOST, Some(&proof)).cookie("service_worker");
    let article = proving_ground.get_with_cookie(&url, cookie.as_deref());

    let record = json!({
        "id": between(&article.body, "data-id=\"", "\""),
        "title": between(&article.body, "<h1 class=\"title\">", "</h1>"),
    });
    let answer = proving_ground.judge(&submission(TASK, "success", &[record]));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(
        observed(&proving_ground),
        [
            (json!(TASK), json!("no_service_worker")),
            (json!(TASK), json!("service_worker")),
        ]
    );
}
