//! A crawl's Session pools per Label, and a blocked pool Session retired
//! and replaced (spec #579's `session_pools`; ADRs 0021, 0088 and 0093),
//! on Foxhound's executable against the Proving ground and Chrome for
//! Testing: on the browser tier each handler's page opens in the browser
//! context of the pool Session that serves it, so a Label that sets
//! `session.source` keeps its cookies apart from the Run's pool, which the
//! Labels that set none share; a Block cause retires its Session, whose
//! replacement is a new Session with a context of its own (the Session
//! ownership and proxy limit conformance cases, the Session pool's half).
//! A Challenge's move to the browser tier is `escalation`'s.

use std::time::Duration;

use goodfetch_protocol::home::RUN_TRACE_CDP_COMMANDS;
use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::attached_with;
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 773;

/// The host of the `access` family's corpus pages.
const HOST: &str = "access.test";

/// An engine whose pages launch Chrome for Testing from the test's cache.
fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    attached_with(env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The settings of a crawl that reaches every host of `proving_ground`,
/// on `tier`, with one Session in each pool at a time.
fn settings(proving_ground: &ProvingGround, tier: &str) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({
        "network": { "resolve": resolve },
        // The egress relay judges a mapping's answers as any other.
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
        "fetch": { "tier": tier },
        "session": { "pool": { "max_size": 1 } },
    })
}

/// A seed of `label` at `path` of the corpus.
fn seed(proving_ground: &ProvingGround, path: &str, label: &str) -> Value {
    json!({ "url": proving_ground.url(HOST, path), "label": label })
}

/// One handler request as the handler saw it: its Label, the pool Session
/// that served it, its tier, and the cookies its page held after the
/// handler set its own.
#[derive(Debug, PartialEq)]
struct Served {
    label: String,
    session: String,
    tier: Value,
    cookies: Value,
}

/// The handler: on a browser page it sets the cookie `set` names for the
/// request's Label, if any, and reads the page's cookies; it answers no
/// Record.
fn handler<'a>(
    seen: &'a mut Vec<Served>,
    set: &'a dyn Fn(&str, usize) -> Option<String>,
) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + 'a {
    move |inside, handle| {
        let label = handle["crawl_request"]["label"]
            .as_str()
            .unwrap()
            .to_owned();
        let cookies = if handle["tier"] == "browser" {
            let read = inside.call(
                "page.evaluate",
                &json!({
                    "page_id": handle["page_id"],
                    "function": "(set) => { if (set) document.cookie = set; return document.cookie; }",
                    "arg": set(&label, seen.len()),
                }),
            );
            read["result"]["value"].clone()
        } else {
            Value::Null
        };
        seen.push(Served {
            label,
            session: handle["session"]["id"].as_str().unwrap().to_owned(),
            tier: handle["tier"].clone(),
            cookies,
        });
        json!({ "result": { "records": [] } })
    }
}

fn served(label: &str, session: &str, tier: &str, cookies: &str) -> Served {
    Served {
        label: label.to_owned(),
        session: session.to_owned(),
        tier: json!(tier),
        cookies: json!(cookies),
    }
}

/// A Label that sets `session.source` (here `null`, a fresh Session
/// whatever the Run's) draws from a pool of its own: the cookie its page
/// set is in its own Session only, while the two Labels that set none
/// share the Run's pool and its cookies.
///
/// Covers: story:5-136, setting:session.source
#[test]
fn a_label_with_a_session_source_keeps_its_cookies_in_its_own_pool_and_labels_without_one_share_the_runs()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let set = |label: &str, served: usize| (served < 2).then(|| format!("who={label}"));

    let mut seen = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "label-pools",
        &[
            seed(&proving_ground, "/corpus/article?n=1", "account"),
            seed(&proving_ground, "/corpus/article?n=2", "public"),
            seed(&proving_ground, "/corpus/article?n=3", "account"),
            seed(&proving_ground, "/corpus/article?n=4", "other"),
        ],
        &json!({
            "account": { "settings": { "session": { "source": null } } },
            "public": {},
            "other": {},
        }),
        &settings(&proving_ground, "browser"),
        &mut handler(&mut seen, &set),
    );
    assert!(engine.finished());

    assert_eq!(
        (seen, &finished["result"]["lifecycle"]),
        (
            vec![
                served("account", "account.session-1", "browser", "who=account"),
                served("public", "session-1", "browser", "who=public"),
                served("account", "account.session-1", "browser", "who=account"),
                served("other", "session-1", "browser", "who=public"),
            ],
            &json!("finished")
        )
    );
}

/// A 403 the engine meets on a pool Session's page retires that Session:
/// its replacement is a new Session at the next proxy tier, whose browser
/// context holds none of the cookies the blocked one held.
///
/// Covers: story:5-138
#[test]
fn a_blocked_pool_session_is_retired_and_its_replacement_is_a_new_session_without_its_cookies() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let set = |_: &str, served: usize| (served == 0).then(|| "kept=1".to_owned());

    let mut seen = Vec::new();
    let mut sessions = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "retired",
        &[
            seed(&proving_ground, "/corpus/article", "page"),
            seed(&proving_ground, "/corpus/ip-block", "page"),
            seed(&proving_ground, "/corpus/vendor-script", "page"),
        ],
        &json!({ "page": {} }),
        &settings(&proving_ground, "browser"),
        &mut |inside, handle| {
            sessions.push(handle["session"].clone());
            handler(&mut seen, &set)(inside, handle)
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            seen,
            sessions[1].clone(),
            contexts(&env, "retired")
                .into_iter()
                .take(3)
                .collect::<Vec<_>>(),
            &finished["result"]["lifecycle"]
        ),
        (
            vec![
                served("page", "session-1", "browser", "kept=1"),
                served("page", "session-2", "browser", ""),
            ],
            json!({ "id": "session-2", "proxy_tier": "datacenter", "revalidate": false }),
            vec!["created", "disposed", "created"],
            &json!("finished")
        )
    );
}

/// Each browser context the Run `run_id` created or disposed, in order, by
/// its CDP trace.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's CDP trace from outside the engine, on no async worker"
)]
fn contexts(env: &TestEnv, run_id: &str) -> Vec<&'static str> {
    let file = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_CDP_COMMANDS);
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(
            |line| match serde_json::from_str::<Value>(line).unwrap()["method"].as_str() {
                Some("Target.createBrowserContext") => Some("created"),
                Some("Target.disposeBrowserContext") => Some("disposed"),
                _ => None,
            },
        )
        .collect()
}
