//! Seeds added to a running crawl on Foxhound's executable over stdio
//! (spec #581's seam: the protocol; ADR 0094; CO-02, CO-03, RT-12):
//! `run.add_seed` is committed at once and answered as an enqueue is,
//! bound to its attempt through a handler's context; `run.add_seeds` takes
//! a stream of seeds under the credits the engine grants, answering each
//! in order, so a million seeds never sit in the engine's memory; and a
//! keep-alive crawl with nothing to do takes seeds from both.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::cell::RefCell;
use std::time::Duration;

use goodfetch_test_client::Attached;
use goodfetch_test_support::{Suite, TestEnv};
use serde_json::{Value, json};

use crate::frontier::{added, crawl, not_added};
use crate::site::Site;

/// The handler of these crawls: one Record naming each page.
fn record_the_page(params: &Value) -> Value {
    json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": params["crawl_request"]["url"] } }] } })
}

/// Starts the keep-alive crawl `run_id` from `seed`.
fn start_keep_alive(engine: &mut Attached, run_id: &str, seed: &str) {
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": [{ "url": seed, "label": "page" }],
            "settings": { "crawl": { "keep_alive": true } },
        }),
        &mut |_| None,
    );
    assert_eq!(started["result"]["run_id"], run_id, "{started}");
}

/// `run.add_seed` of `url` to `run_id`, from outside any attempt: its
/// `Enqueued`, or its Error code.
fn add_seed(
    engine: &mut Attached,
    run_id: &str,
    url: &str,
    handler: &mut impl FnMut(&Value) -> Option<Value>,
) -> Value {
    let reply = engine.call(
        "add",
        "run.add_seed",
        &json!({ "run_id": run_id, "seed": { "url": url, "label": "page" } }),
        handler,
    );
    reply
        .get("result")
        .unwrap_or_else(|| &reply["error"]["data"]["code"])
        .clone()
}

/// The URL and kind of each outcome of `run_id`, in commit order.
fn outcomes(engine: &mut Attached, run_id: &str) -> Vec<(Value, Value)> {
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    read["result"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| (outcome["request"]["url"].clone(), outcome["kind"].clone()))
        .collect()
}

/// A seed added to a running crawl is committed at once and answered as an
/// enqueue is: added with its ID and its URL as parsed, or not added with
/// `duplicate`, `unsupported_scheme` or `invalid_url`; the handler gets
/// the added one while the crawl runs, and a crawl this engine does not run
/// answers `run_not_found`.
///
/// Covers: story:6-208, operation:run.add_seed
#[test]
fn a_seed_added_to_a_running_crawl_is_committed_at_once_and_answered_as_an_enqueue_is() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    start_keep_alive(&mut engine, "adding", &site.page("/a"));
    let pages = RefCell::new(Vec::new());
    let mut handler = |params: &Value| {
        pages
            .borrow_mut()
            .push(params["crawl_request"]["url"].clone());
        Some(record_the_page(params))
    };

    let answers = [
        add_seed(
            &mut engine,
            "adding",
            &site.page("/b").replace("shop.test", "SHOP.test"),
            &mut handler,
        ),
        add_seed(&mut engine, "adding", &site.page("/b#top"), &mut handler),
        add_seed(&mut engine, "adding", "ftp://shop.test/c", &mut handler),
        add_seed(&mut engine, "adding", "c", &mut handler),
        add_seed(&mut engine, "elsewhere", &site.page("/d"), &mut handler),
    ];
    serve_until(&mut engine, &mut handler, || pages.borrow().len() == 2);
    let mut handled_while_running = pages.borrow().clone();
    handled_while_running.sort_by_key(Value::to_string);
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "adding" }),
        &mut handler,
    );
    let mut outcomes = outcomes(&mut engine, "adding");
    outcomes.sort_by_key(|(url, _)| url.to_string());
    assert!(engine.finished());

    assert_eq!(
        answers,
        [
            added("2", &site.page("/b")),
            not_added("duplicate", &site.page("/b#top")),
            not_added("unsupported_scheme", "ftp://shop.test/c"),
            not_added("invalid_url", "c"),
            json!("run_not_found"),
        ]
    );
    assert_eq!(
        (
            handled_while_running,
            &report["result"]["lifecycle"],
            outcomes
        ),
        (
            vec![json!(site.page("/a")), json!(site.page("/b"))],
            &json!("finished"),
            vec![
                (json!(site.page("/a")), json!("succeeded")),
                (json!(site.page("/b")), json!("succeeded")),
            ],
        )
    );
}

/// A handler's `ctx.run.addSeed` names its attempt: the seed is added
/// while the attempt holds its lease, at depth 0 like any seed, and the
/// same call from an attempt that already ended fails with
/// `attempt_ended`, adding nothing.
///
/// Covers: story:6-208, operation:run.add_seed, error:attempt_ended
#[test]
fn a_seed_added_through_a_handlers_context_is_bound_to_its_attempt() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let mut first_attempt: Option<Value> = None;
    let mut answers = Vec::new();

    crawl(
        &mut engine,
        "bound",
        &[json!({ "url": site.page("/a"), "label": "page" })],
        &mut |inside, handle| {
            let attempt = first_attempt.get_or_insert_with(|| {
                json!({
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                })
            });
            if answers.len() < 2 {
                let url = if answers.is_empty() {
                    site.page("/a-seeded")
                } else {
                    site.page("/late")
                };
                let reply = inside.call(
                    "run.add_seed",
                    &json!({
                        "run_id": "bound",
                        "seed": { "url": url, "label": "page" },
                        "attempt": attempt,
                    }),
                );
                answers.push(
                    reply
                        .get("result")
                        .unwrap_or_else(|| &reply["error"]["data"]["code"])
                        .clone(),
                );
            }
            record_the_page(handle)
        },
    );
    let outcomes = outcomes(&mut engine, "bound");
    assert!(engine.finished());

    assert_eq!(
        (answers, outcomes),
        (
            vec![added("2", &site.page("/a-seeded")), json!("attempt_ended"),],
            vec![
                (json!(site.page("/a")), json!("succeeded")),
                (json!(site.page("/a-seeded")), json!("succeeded")),
            ],
        )
    );
}

/// The id of the `run.add_seeds` calls below.
const STREAM: &str = "stream";

/// What came for a `run.add_seeds` call: its credits, its answers and its
/// reply, in order.
#[derive(Debug, PartialEq)]
enum Came {
    Credit(u64),
    Answers(Vec<Value>),
    Reply(Value),
}

/// The next thing that comes for the `run.add_seeds` call, each handler
/// request meanwhile answered by `handler`, or left unanswered when it
/// answers `None`.
fn next_for_stream(
    engine: &mut Attached,
    handler: &mut impl FnMut(&Value) -> Option<Value>,
) -> Came {
    loop {
        let message = engine.next().expect("the engine closed in a stream");
        if message["method"] == "crawl_request.handle" {
            if let Some(mut reply) = handler(&message["params"]) {
                reply["jsonrpc"] = json!("2.0");
                reply["id"] = message["id"].clone();
                engine.send(&reply);
            }
        } else if message["method"] == "$/credit" && message["params"]["id"] == STREAM {
            return Came::Credit(message["params"]["credits"].as_u64().unwrap());
        } else if message["method"] == "$/enqueued" && message["params"]["id"] == STREAM {
            return Came::Answers(message["params"]["answers"].as_array().unwrap().clone());
        } else if message["id"] == STREAM {
            return Came::Reply(message);
        }
    }
}

/// Sends the `run.add_seeds` call's next batch of seeds.
fn send_seeds(engine: &mut Attached, seeds: &[Value], end: bool) {
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/seeds",
        "params": { "id": STREAM, "seeds": seeds, "end": end },
    }));
}

/// Answers each handler request with `handler` until `done`, before the
/// crawl is let finish: the engine's stdout deadline fails a crawl that
/// never hands its work over.
fn serve_until(
    engine: &mut Attached,
    handler: &mut impl FnMut(&Value) -> Option<Value>,
    done: impl Fn() -> bool,
) {
    while !done() {
        let message = engine.next().expect("the engine closed while serving");
        if message["method"] == "crawl_request.handle"
            && let Some(mut reply) = handler(&message["params"])
        {
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = message["id"].clone();
            engine.send(&reply);
        }
    }
}

/// The seed of `url`.
fn seed(url: &str) -> Value {
    json!({ "url": url, "label": "page" })
}

/// A keep-alive crawl whose one seed was refused has nothing to do and
/// idles, quiet and still running, so a seed reaches it; it takes a seed
/// from `run.add_seed` and one streamed through `run.add_seeds`, and hands
/// each to the handler as it comes, before anything lets it finish; it
/// finishes `complete` once `run.finish` lets it (golden scenario 12).
///
/// Covers: story:6-210, story:3-128, row:RT-12, setting:crawl.keep_alive, operation:run.add_seed, operation:run.add_seeds
#[test]
fn a_keep_alive_crawl_with_nothing_to_do_takes_seeds_from_add_seed_and_add_seeds() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    start_keep_alive(&mut engine, "service", "ftp://shop.test/");
    let idle = engine.is_quiet_for(Duration::from_millis(500));
    let pages = RefCell::new(Vec::new());
    let mut handler = |params: &Value| {
        pages
            .borrow_mut()
            .push(params["crawl_request"]["url"].clone());
        Some(record_the_page(params))
    };

    let one = add_seed(&mut engine, "service", &site.page("/one"), &mut handler);
    serve_until(&mut engine, &mut handler, || pages.borrow().len() == 1);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": STREAM, "method": "run.add_seeds",
        "params": { "run_id": "service" },
    }));
    let credit = next_for_stream(&mut engine, &mut handler);
    send_seeds(&mut engine, &[seed(&site.page("/two"))], true);
    let streamed = [
        next_for_stream(&mut engine, &mut handler),
        next_for_stream(&mut engine, &mut handler),
    ];
    serve_until(&mut engine, &mut handler, || pages.borrow().len() == 2);
    let handled_before_finish = pages.borrow().clone();
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "service" }),
        &mut handler,
    );
    assert!(engine.finished());

    assert_eq!(
        (idle, one, credit),
        (true, added("1", &site.page("/one")), Came::Credit(256))
    );
    assert_eq!(
        streamed[0],
        Came::Answers(vec![added("2", &site.page("/two"))])
    );
    let Came::Reply(reply) = &streamed[1] else {
        panic!("the stream did not end: {:?}", streamed[1]);
    };
    assert_eq!(
        (
            &reply["result"],
            handled_before_finish,
            &report["result"]["lifecycle"],
            &report["result"]["completeness"],
        ),
        (
            &json!({ "added": 1, "not_added": 0 }),
            vec![json!(site.page("/one")), json!(site.page("/two"))],
            &json!("finished"),
            &json!("complete"),
        )
    );
}

/// `run.add_seeds` takes no more seeds than the credits it granted: 256 at
/// once, and as many again once it answered a batch, each seed's answer in
/// the batch's order; a batch beyond the credits fails the call with
/// `invalid_params`.
///
/// Covers: story:6-209, operation:run.add_seeds
#[test]
fn add_seeds_answers_each_seed_in_order_under_the_credits_it_grants() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    start_keep_alive(&mut engine, "credits", &site.page("/0"));
    let mut unanswered = |_: &Value| None;
    engine.send(&json!({
        "jsonrpc": "2.0", "id": STREAM, "method": "run.add_seeds",
        "params": { "run_id": "credits" },
    }));
    let first_credit = next_for_stream(&mut engine, &mut unanswered);
    let urls: Vec<String> = (1..=256)
        .map(|n| match n {
            100 => site.page("/7"),
            200 => "mailto:owner@shop.test".to_owned(),
            n => site.page(&format!("/{n}")),
        })
        .collect();
    let seeds: Vec<Value> = urls.iter().map(|url| seed(url)).collect();
    send_seeds(&mut engine, &seeds, false);
    let answers = next_for_stream(&mut engine, &mut unanswered);
    let next_credit = next_for_stream(&mut engine, &mut unanswered);
    send_seeds(&mut engine, &vec![seed(&site.page("/x")); 257], false);
    let refused = next_for_stream(&mut engine, &mut unanswered);
    let exited = engine.finished();

    let Came::Answers(answers) = answers else {
        panic!("no answers came: {answers:?}");
    };
    let kinds: Vec<(&Value, &Value, &Value)> = answers
        .iter()
        .map(|answer| (&answer["kind"], &answer["url"], &answer["reason"]))
        .collect();
    let (added_kind, not_added_kind) = (json!("added"), json!("not_added"));
    let (duplicate, unsupported, none) =
        (json!("duplicate"), json!("unsupported_scheme"), Value::Null);
    let given: Vec<Value> = urls.iter().map(|url| json!(url)).collect();
    let expected: Vec<(&Value, &Value, &Value)> = given
        .iter()
        .enumerate()
        .map(|(index, url)| match index + 1 {
            100 => (&not_added_kind, url, &duplicate),
            200 => (&not_added_kind, url, &unsupported),
            _ => (&added_kind, url, &none),
        })
        .collect();
    let ids: Vec<u64> = answers
        .iter()
        .filter_map(|answer| answer["id"].as_str())
        .map(|id| id.parse().unwrap())
        .collect();
    assert_eq!(
        (first_credit, &kinds, next_credit, exited),
        (Came::Credit(256), &expected, Came::Credit(256), true)
    );
    assert_eq!(ids, (2..=255).collect::<Vec<u64>>());
    let Came::Reply(refused) = refused else {
        panic!("the call was not refused: {refused:?}");
    };
    assert_eq!(
        refused["error"]["data"]["code"], "invalid_params",
        "{refused}"
    );
}

/// The engine's resident memory in bytes, as `ps` reads it; `None` on
/// Windows, which has no `ps`.
fn resident_bytes(env: &TestEnv, pid: u32) -> Option<u64> {
    if cfg!(windows) {
        return None;
    }
    let output = env
        .process_command("/bin/ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let kibibytes: u64 = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    Some(kibibytes * 1024)
}

/// A million seeds streamed through `run.add_seeds` (a hundred thousand
/// outside the Release suite) are each answered in order, while the engine
/// holds no more than its credits of them: on macOS and Linux, its resident
/// memory once they are all added is within 64 MiB of what it was after the
/// first tenth, where buffering the stream would take hundreds.
///
/// Covers: story:6-209, operation:run.add_seeds
#[test]
fn a_million_seeds_stream_through_add_seeds_with_the_engines_memory_bounded_by_its_credits() {
    let env = TestEnv::new();
    let count: u64 = match env.suite() {
        Suite::Release => 1_000_000,
        _ => 100_000,
    };
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    start_keep_alive(&mut engine, "million", &site.url("many.test", "/0"));
    let mut unanswered = |_: &Value| None;
    engine.send(&json!({
        "jsonrpc": "2.0", "id": STREAM, "method": "run.add_seeds",
        "params": { "run_id": "million" },
    }));
    let mut credits = 0;
    let mut next = 1;
    let mut answered = 0;
    let mut out_of_order = 0;
    let mut after_a_tenth: Option<Option<u64>> = None;
    let mut highest: Option<u64> = None;
    while answered < count {
        if credits > 0 && next <= count {
            let batch: Vec<Value> = (next..(next + credits).min(count + 1))
                .map(|n| seed(&site.url("many.test", &format!("/{}/{n}", n % 64))))
                .collect();
            next += batch.len() as u64;
            credits -= batch.len() as u64;
            send_seeds(&mut engine, &batch, next > count);
            continue;
        }
        match next_for_stream(&mut engine, &mut unanswered) {
            Came::Credit(granted) => credits += granted,
            Came::Answers(answers) => {
                for answer in answers {
                    answered += 1;
                    let n: u64 = answer["url"]
                        .as_str()
                        .unwrap()
                        .rsplit('/')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    if n != answered || answer["kind"] != "added" {
                        out_of_order += 1;
                    }
                }
                if after_a_tenth.is_none() && answered >= count / 10 {
                    after_a_tenth = Some(resident_bytes(&env, engine.pid()));
                }
                if answered % 50_000 == 0 {
                    highest = highest.max(resident_bytes(&env, engine.pid()));
                }
            }
            Came::Reply(reply) => panic!("the stream ended early: {reply}"),
        }
    }
    let reply = next_for_stream(&mut engine, &mut unanswered);
    highest = highest.max(resident_bytes(&env, engine.pid()));
    let exited = engine.finished();

    assert_eq!(
        (out_of_order, exited, reply),
        (
            0,
            true,
            Came::Reply(
                json!({ "jsonrpc": "2.0", "id": STREAM, "result": { "added": count, "not_added": 0 } })
            )
        )
    );
    if let (Some(Some(after_a_tenth)), Some(highest)) = (after_a_tenth, highest) {
        assert!(
            highest < after_a_tenth + 64 * 1024 * 1024,
            "the engine grew from {after_a_tenth} to {highest} bytes"
        );
    }
}
