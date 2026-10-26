//! A Client acting for a model only narrows the domain policy on Chromium
//! over stdio (spec #563, story 4-4, G5): user configuration sets
//! `egress.allowed_domains` and `egress.blocked_domains`, and a model's
//! `run.start` that lists fewer allowed hosts narrows them, so its page is
//! refused a host configuration allows, while one that lists more allowed
//! hosts or drops a blocked one is `setting_locked` and starts nothing.
//! The Proving ground's request record is the oracle: a refused page
//! reaches no site.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::page::{BROWSER_DEADLINE, opened};
use crate::run::call;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 624;

/// Writes user configuration that maps the Proving ground's hosts to its
/// address, lists them in `egress.private_ranges` (a model may not), allows
/// `catalog.test` and `shop.test` and blocks `elsewhere.test`, after the
/// settings every test's home starts with, which keep the engine's JSON log
/// and so the test's exercise record.
fn configure(env: &TestEnv, proving_ground: &ProvingGround) {
    let resolve = &proving_ground.description().resolve;
    let hosts: Vec<String> = resolve.keys().map(|host| format!("{host:?}")).collect();
    let mapping: Vec<String> = resolve
        .iter()
        .map(|(host, address)| format!("{host:?} = [{address:?}]"))
        .collect();
    env.user_configuration(&format!(
        "egress.allowed_domains = [\"catalog.test\", \"shop.test\"]\negress.blocked_domains = [\"elsewhere.test\"]\negress.private_ranges = [{}]\nnetwork.resolve = {{ {} }}\n",
        hosts.join(", "),
        mapping.join(", ")
    ));
}

/// An Attached engine whose Browser cache is `cache`, greeted by a Client
/// acting for a model (the `mcp` surface).
fn for_a_model<'env>(env: &'env TestEnv, cache: &Path) -> Attached<'env> {
    let mut engine = Attached::start_waiting(env, BROWSER_DEADLINE, |command| {
        command.env(BROWSERS_PATH_VARIABLE, cache);
    });
    let mut greeting = hello("collie", "0.1");
    greeting["params"]["surface"] = json!("mcp");
    engine.send(&greeting);
    let _ = engine.next();
    engine
}

/// The answer to `run.start` of `run_id` with `egress` as its `egress`
/// settings.
fn start(engine: &mut Attached<'_>, id: i64, run_id: &str, egress: &Value) -> Value {
    call(
        engine,
        id,
        "run.start",
        &json!({ "run_id": run_id, "settings": { "egress": egress } }),
    )
}

/// A refusal's code, key and the value of the limit that holds it.
fn locked(answered: &Value) -> (Value, Value, Value) {
    let data = &answered["error"]["data"];
    (
        data["code"].clone(),
        data["key"].clone(),
        data["limit"]["value"].clone(),
    )
}

/// A Client acting for a model that narrows `egress.allowed_domains` to
/// `catalog.test` loads it and is refused `shop.test`, which configuration
/// allows, with `egress_refused` naming the setting and no request reaching
/// it; the same model's `run.start` that adds a host to the allowed list,
/// or drops `elsewhere.test` from the blocked one, is `setting_locked`
/// naming the key and the configured value that holds it, and starts no
/// Run (the narrowing value is the positive control).
///
/// Covers: story:4-4, error:egress_refused, error:setting_locked, setting:egress.allowed_domains, setting:egress.blocked_domains, operation:run.start, operation:page.goto
#[test]
fn a_client_acting_for_a_model_narrows_the_domain_policy_and_never_widens_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    configure(&env, &proving_ground);
    let mut engine = for_a_model(&env, &cache);

    let wider = start(
        &mut engine,
        1,
        "wider",
        &json!({ "allowed_domains": ["catalog.test", "shop.test", "scope.test"] }),
    );
    let unblocked = start(
        &mut engine,
        2,
        "unblocked",
        &json!({ "blocked_domains": [] }),
    );
    let narrowed = start(
        &mut engine,
        3,
        "narrowed",
        &json!({ "allowed_domains": ["catalog.test"] }),
    );
    let page = opened(&call(
        &mut engine,
        4,
        "run.new_page",
        &json!({ "run_id": "narrowed" }),
    ))
    .page_id
    .0;
    let goto = |engine: &mut Attached<'_>, id: i64, host: &str| {
        call(
            engine,
            id,
            "page.goto",
            &json!({ "page_id": page, "url": proving_ground.url(host, "/catalog/1?run=narrowed") }),
        )
    };
    let listed = goto(&mut engine, 5, "catalog.test");
    let outside = goto(&mut engine, 6, "shop.test");
    let reached = |host: &str| {
        proving_ground
            .received_requests()
            .unwrap()
            .of_the_case()
            .filter(|request| request["host"] == json!(host))
            .count()
    };
    let (catalog, shop) = (reached("catalog.test"), reached("shop.test"));
    let _ = call(
        &mut engine,
        7,
        "run.finish",
        &json!({ "run_id": "narrowed" }),
    );
    let _ = engine.finish();

    assert_eq!(
        (locked(&wider), locked(&unblocked)),
        (
            (
                json!("setting_locked"),
                json!("egress.allowed_domains"),
                json!(["catalog.test", "shop.test"])
            ),
            (
                json!("setting_locked"),
                json!("egress.blocked_domains"),
                json!(["elsewhere.test"])
            ),
        ),
        "{wider} {unblocked}"
    );
    assert_eq!(
        narrowed["result"]["lifecycle"],
        json!("running"),
        "{narrowed}"
    );
    let refused = &outside["error"]["data"];
    assert_eq!(
        (
            listed["result"]["response"]["status"].clone(),
            catalog > 0,
            refused["code"].clone(),
            refused["rule"].clone(),
            refused["setting"].clone(),
            shop,
        ),
        (
            json!(200),
            true,
            json!("egress_refused"),
            json!("domain_policy"),
            json!("egress.allowed_domains"),
            0
        ),
        "{listed} {outside}"
    );
}
