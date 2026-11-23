//! ADR 0106's send window in a crawl, on Foxhound's executable against the
//! Proving ground on the `http` tier: a body a handler declares through
//! its attempt turns `sent` before the attempt's first fetch hop goes out,
//! a navigation of its served page, and a body another attempt declared
//! stays `open`. The Run's end then cuts short the bodies no `effect.close`
//! judged (ADR 0106): the sent one `uncertain`, the other dropped as never
//! sent.

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::frontier::{Inside, crawl};
use crate::site::Site;

/// `effect.open` of the body `key` through the attempt `handle` gave;
/// answers its reply.
fn declared(inside: &mut Inside<'_>, handle: &Value, key: &str) -> Value {
    inside.call(
        "effect.open",
        &json!({
            "run_id": handle["run_id"],
            "key": key,
            "attempt": {
                "crawl_request_id": handle["crawl_request"]["id"],
                "lease_epoch": handle["lease_epoch"],
            },
        }),
    )
}

/// Each body's key, state and first command in the Run `run_id`, in the
/// order declared.
fn journal(env: &TestEnv, run_id: &str) -> Vec<(String, String, Option<String>)> {
    Connection::open(HomeLayout::of(env.home()).run_file(run_id))
        .unwrap()
        .prepare("SELECT key, state, first_command FROM effect_journal ORDER BY entry")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The first attempt declares `other` and answers; the second declares
/// `order`, then navigates its served page, whose fetch hop marks `order`
/// sent and leaves `other` open: at the Run's end, `order` is left
/// `uncertain` with its first command, and `other`, never sent, is dropped.
///
/// Covers: operation:effect.open, operation:page.goto
#[test]
fn an_attempts_fetch_hop_marks_its_own_body_sent_and_not_another_attempts() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let seeds = [
        json!({ "url": site.page("/a"), "label": "page" }),
        json!({ "url": site.page("/b"), "label": "page" }),
    ];
    let next = site.page("/1");
    let mut engine = site.engine(&env);

    let mut replies = Vec::new();
    crawl(&mut engine, "window", &seeds, &mut |inside, handle| {
        if replies.is_empty() {
            replies.push(declared(inside, handle, "other"));
        } else {
            replies.push(declared(inside, handle, "order"));
            replies.push(inside.call(
                "page.goto",
                &json!({ "page_id": handle["page_id"], "url": next }),
            ));
        }
        json!({ "result": { "records": [] } })
    });
    assert!(engine.finished());

    assert!(
        replies.len() == 3 && replies.iter().all(|reply| reply.get("result").is_some()),
        "{replies:?}"
    );
    assert_eq!(
        journal(&env, "window"),
        vec![(
            "order".to_owned(),
            "uncertain".to_owned(),
            Some("page.goto".to_owned())
        )]
    );
}
