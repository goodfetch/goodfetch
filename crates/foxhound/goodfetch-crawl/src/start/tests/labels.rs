//! A crawl works only the Labels it declares (ADR 0093): a seed, a
//! sitemap rule, an enqueue, a seed added or a resume naming one it does
//! not declare fails with `label_unknown`, its details naming the Label
//! and those the crawl declares, and a resume that declares a new Label
//! takes it. A resume records its Labels in the run file in place of the
//! Run's, so each works under the settings the latest start declared.

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    POLICY, Reply, answer_all, configure, crawl, default_dependencies, delivered, dependencies,
    first_only, page_of, paused_after_its_first_page, records, run_file, run_folder, seeds,
    work_items,
};
use crate::policy::CrawlPolicy;

/// One handler at a time, so the work items go one after another.
const ONE_AT_A_TIME: CrawlPolicy = CrawlPolicy {
    handler_concurrency: 1,
    ..POLICY
};

/// The code of the error `answer` carries, and its details: its data's
/// other fields but `retryable`.
fn refusal(answer: &Value) -> (Value, Value) {
    let mut details = answer["error"]["data"].clone();
    let code = details["code"].take();
    if let Some(fields) = details.as_object_mut() {
        fields.retain(|field, _| field != "code" && field != "retryable");
    }
    (code, details)
}

/// Covers: error:label_unknown, operation:crawl.start
#[rstest]
#[case::a_request_seed(json!({ "url": "https://shop.test/book/1", "label": "book" }))]
#[case::a_sitemap_rule(json!({
    "sitemap": "https://shop.test/sitemap.xml",
    "rules": [{ "pattern": "https://shop.test/book/*", "label": "book" }],
}))]
fn a_seed_naming_a_label_the_crawl_does_not_declare_fails_the_start_before_its_run_begins(
    #[case] seed: Value,
) {
    let env = TestEnv::new();

    let answer = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "refused",
                        "seeds": [{ "url": "https://shop.test/page/1", "label": "page" }, seed],
                        "labels": { "page": {}, "author": {} },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await
        },
    );

    assert_eq!(
        (refusal(&answer), run_folder(&env, "refused").exists()),
        (
            (
                json!("label_unknown"),
                json!({ "label": "book", "declared": ["author", "page"] }),
            ),
            false,
        ),
        "{answer}"
    );
}

/// Covers: error:label_unknown, operation:crawl_request.enqueue, operation:crawl_request.enqueue_links, operation:run.add_seed
#[test]
fn an_enqueue_or_a_seed_added_naming_a_label_the_crawl_does_not_declare_fails_and_adds_nothing() {
    let env = TestEnv::new();

    let (replies, handled) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "enqueuing", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |params: &Value| {
                if params["attempt"] != 1 || !page_of(params).ends_with("/page/1") {
                    return answer_all(params);
                }
                let from = json!({
                    "run_id": params["run_id"],
                    "crawl_request_id": params["crawl_request"]["id"],
                    "lease_epoch": params["lease_epoch"],
                    "label": "book",
                });
                let mut enqueue = from.clone();
                enqueue["url"] = json!("https://shop.test/book/1");
                let mut links = from;
                links["locator"] = json!({ "page_id": "page-1", "selector": "a" });
                let seed = json!({
                    "run_id": params["run_id"],
                    "seed": { "url": "https://shop.test/book/2", "label": "book" },
                });
                Reply::Calls(
                    vec![
                        ("crawl_request.enqueue", enqueue),
                        ("crawl_request.enqueue_links", links),
                        ("run.add_seed", seed),
                    ],
                    Box::new(answer_all(params)),
                )
            };
            client
                .call("run.finish", json!({ "run_id": "enqueuing" }), &mut handler)
                .await;
            (client.replies.clone(), delivered(client))
        },
    );

    let unknown = (
        json!("label_unknown"),
        json!({ "label": "book", "declared": ["page"] }),
    );
    assert_eq!(
        (replies.iter().map(refusal).collect::<Vec<_>>(), handled),
        (
            vec![unknown.clone(), unknown.clone(), unknown],
            vec![("https://shop.test/page/1".to_owned(), 1)],
        ),
        "{replies:?}"
    );
}

/// Answers each handled page, the second enqueuing a `book`.
fn enqueue_a_book(params: &Value) -> Reply {
    if !page_of(params).ends_with("/page/2") {
        return answer_all(params);
    }
    let enqueue = json!({
        "run_id": params["run_id"],
        "crawl_request_id": params["crawl_request"]["id"],
        "lease_epoch": params["lease_epoch"],
        "url": "https://shop.test/book/1",
        "label": "book",
    });
    Reply::Calls(
        vec![("crawl_request.enqueue", enqueue)],
        Box::new(answer_all(params)),
    )
}

/// The start of a crawl of two `page` seeds under the Run ID `paused`,
/// declaring `labels`.
fn paused_start(labels: &Value) -> Value {
    json!({ "run_id": "paused", "seeds": seeds(2), "labels": labels })
}

/// A start under the ID of a paused Run whose pending request names a
/// Label the crawl no longer declares is refused before any work: nothing
/// is delivered and the Run's work items stay as they were (ADR 0102).
///
/// Covers: error:label_unknown, operation:crawl.start
#[test]
fn a_resume_whose_pending_requests_name_a_label_no_longer_declared_is_refused_before_any_work() {
    let env = TestEnv::new();

    let (refused, handled, before, after) = crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, RunStateOptions::default())
        },
        async |client| {
            paused_after_its_first_page(client, &env, paused_start(&json!({ "page": {} }))).await;
            let before = work_items(&run_file(&env, "paused"));
            client.handled.clear();
            let refused = client
                .call(
                    "crawl.start",
                    paused_start(&json!({ "book": {} })),
                    &mut answer_all,
                )
                .await;
            let after = work_items(&run_file(&env, "paused"));
            (refused, delivered(client), before, after)
        },
    );

    assert_eq!(
        (refusal(&refused), handled, after),
        (
            (
                json!("label_unknown"),
                json!({ "label": "page", "declared": ["book"] }),
            ),
            Vec::new(),
            before,
        ),
        "{refused}"
    );
}

/// A resume passes a Run whose failed or blocked request names a Label
/// the crawl no longer declares, as only pending requests are judged; the
/// live crawl's `run.retry_failed` then refuses to re-admit it, failing
/// with `label_unknown`, its details naming the Label and those the crawl
/// declares, and re-admits nothing (ADRs 0093 and 0102). Once the Run
/// ended, the call refuses it the same against the Labels its run file
/// records, before it reopens the Run (#2825). A blocked request is
/// judged with `blocked` alone.
///
/// Covers: error:label_unknown, operation:run.retry_failed
#[rstest]
#[case::failed(false)]
#[case::blocked(true)]
fn a_retry_of_a_request_whose_label_the_crawl_no_longer_declares_is_refused_live_and_ended(
    #[case] blocked: bool,
) {
    let env = TestEnv::new();
    let mut ends = |_: &Value| {
        if blocked {
            Reply::Blocked("rate_limit")
        } else {
            Reply::Fails("handler_failed", false)
        }
    };
    let keep_alive = json!({ "crawl": { "keep_alive": true } });
    // A resume does not check its seeds (ADR 0102).
    let seeds = json!([{ "url": "https://shop.test/old", "label": "old" }]);

    let (ended, resumed, refused, before, after, finished) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "old": {}, "page": {} },
                        "run_id": "dropped",
                        "seeds": seeds,
                        "settings": keep_alive,
                    }),
                    &mut ends,
                )
                .await;
            let ended = client
                .call("run.outcomes", json!({ "run_id": "dropped" }), &mut ends)
                .await;
            client
                .call("run.pause", json!({ "run_id": "dropped" }), &mut ends)
                .await;
            let resumed = client
                .call(
                    "crawl.start",
                    json!({
                        "labels": { "page": {} },
                        "run_id": "dropped",
                        "seeds": seeds,
                        "settings": keep_alive,
                    }),
                    &mut ends,
                )
                .await;
            let before = work_items(&run_file(&env, "dropped"));
            let refused = client
                .call(
                    "run.retry_failed",
                    json!({ "run_id": "dropped", "blocked": blocked }),
                    &mut ends,
                )
                .await;
            let after = work_items(&run_file(&env, "dropped"));
            client
                .call("run.finish", json!({ "run_id": "dropped" }), &mut ends)
                .await;
            let report = json!({ "run_id": "dropped" });
            let reported = client.call("run.report", report.clone(), &mut ends).await;
            let ended_refused = client
                .call(
                    "run.retry_failed",
                    json!({ "run_id": "dropped", "blocked": blocked }),
                    &mut ends,
                )
                .await;
            let finished = (
                reported,
                ended_refused,
                work_items(&run_file(&env, "dropped")),
                client.call("run.report", report, &mut ends).await,
            );
            (ended, resumed, refused, before, after, finished)
        },
    );
    let (reported, ended_refused, ended_after, reported_after) = finished;

    let label_unknown = (
        json!("label_unknown"),
        json!({ "label": "old", "declared": ["page"] }),
    );
    assert_eq!(
        (
            &ended["result"]["outcomes"][0]["kind"],
            resumed.get("error"),
            refusal(&refused),
            &after,
            &reported["result"]["lifecycle"],
            refusal(&ended_refused),
            &ended_after,
            &reported_after["result"],
        ),
        (
            &json!(if blocked { "blocked" } else { "failed" }),
            None,
            label_unknown.clone(),
            &before,
            &json!("finished"),
            label_unknown,
            &before,
            &reported["result"],
        ),
        "{refused}\n{ended_refused}"
    );
}

/// A start under the ID of a paused Run that declares a new Label beside
/// those its pending requests name resumes the Run, which takes that new
/// Label's enqueue (ADR 0102).
///
/// Covers: operation:crawl.start, operation:run.resume
#[test]
fn a_resume_that_declares_a_new_label_takes_its_enqueue() {
    let env = TestEnv::new();

    let (started, replies) = crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, RunStateOptions::default())
        },
        async |client| {
            paused_after_its_first_page(client, &env, paused_start(&json!({ "page": {} }))).await;
            let started = client
                .call(
                    "crawl.start",
                    paused_start(&json!({ "page": {}, "book": {} })),
                    &mut first_only,
                )
                .await;
            client
                .call(
                    "run.resume",
                    json!({ "run_id": "paused" }),
                    &mut enqueue_a_book,
                )
                .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "paused" }),
                    &mut enqueue_a_book,
                )
                .await;
            (started, client.replies.clone())
        },
    );

    let pages: Vec<Value> = records(&run_file(&env, "paused"))
        .into_iter()
        .map(|(_, _, _, data)| data)
        .collect();
    assert_eq!(
        (
            started["result"]["run_id"].clone(),
            replies
                .iter()
                .map(|reply| reply.get("error").cloned())
                .collect::<Vec<_>>(),
            pages,
        ),
        (
            json!("paused"),
            vec![None],
            vec![
                json!({ "page": "https://shop.test/page/1" }),
                json!({ "page": "https://shop.test/page/2" }),
                json!({ "page": "https://shop.test/book/1" }),
            ],
        ),
        "{started} {replies:?}"
    );
}

/// A `book` Label with settings of its own, each unlike the Run's: its own
/// forced tier, the encoding `windows-1252`, and a Session pool of its
/// own, whose Sessions open fresh.
fn book() -> Value {
    json!({
        "settings": {
            "fetch": { "tier": "http", "encoding": "windows-1252" },
            "session": { "source": null },
        },
    })
}

/// An `author` Label whose Page check expects an element no page of the
/// harness's site holds.
fn author() -> Value {
    json!({
        "settings": {
            "session": { "page_check": { "expected_page": { "kind": "css", "selector": "#absent" } } },
        },
    })
}

/// Answers each handled page, the second enqueuing a `book` and an
/// `author`.
fn enqueue_a_book_and_an_author(params: &Value) -> Reply {
    if !page_of(params).ends_with("/page/2") {
        return answer_all(params);
    }
    let enqueue = |url: &str, label: &str| {
        json!({
            "run_id": params["run_id"],
            "crawl_request_id": params["crawl_request"]["id"],
            "lease_epoch": params["lease_epoch"],
            "url": url,
            "label": label,
        })
    };
    Reply::Calls(
        vec![
            (
                "crawl_request.enqueue",
                enqueue("https://shop.test/book/1", "book"),
            ),
            (
                "crawl_request.enqueue",
                enqueue("https://shop.test/author/1", "author"),
            ),
        ],
        Box::new(answer_all(params)),
    )
}

/// Each `fetch_tier` entry of the Provenance log of `run_id`: its Label,
/// the layer that forced its tier, and the encoding its document was read
/// in, with where that encoding came from.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's Provenance log once its engine stopped, on no async worker"
)]
fn fetches(env: &TestEnv, run_id: &str) -> Vec<Value> {
    std::fs::read_to_string(run_folder(env, run_id).join(RUN_TRACE_PROVENANCE))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["action"] == "fetch_tier")
        .map(|entry| {
            json!([
                entry["label"],
                entry["forced_by"],
                entry["encoding"],
                entry["encoding_source"]
            ])
        })
        .collect()
}

/// The Session pools of the Run `run_id` that opened a Session, the Run's
/// own named `""`.
fn pools(env: &TestEnv, run_id: &str) -> Vec<String> {
    run_file(env, run_id)
        .prepare("SELECT DISTINCT pool FROM pool_sessions ORDER BY pool")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The run file's declared Labels, each name with its declaration.
fn declared_labels(env: &TestEnv, run_id: &str) -> Vec<(String, Value)> {
    run_file(env, run_id)
        .prepare("SELECT name, declaration FROM labels ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))
        .unwrap()
        .map(|row| {
            let (name, declaration) = row.unwrap();
            (name, serde_json::from_str(&declaration).unwrap())
        })
        .collect()
}

/// A Label a resume declares first works under its own settings, not the
/// Run's: its `fetch.tier`, its `fetch.encoding`, its Session pool and its
/// `session.page_check` (#2825).
///
/// Covers: operation:crawl.start, setting:fetch.tier, setting:fetch.encoding, setting:session.source, setting:session.page_check
#[test]
fn a_label_a_resume_declares_first_works_under_its_own_settings() {
    let env = TestEnv::new();

    crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, RunStateOptions::default())
        },
        async |client| {
            paused_after_its_first_page(client, &env, paused_start(&json!({ "page": {} }))).await;
            client
                .call(
                    "crawl.start",
                    paused_start(&json!({ "page": {}, "book": book(), "author": author() })),
                    &mut first_only,
                )
                .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "paused" }),
                    &mut enqueue_a_book_and_an_author,
                )
                .await;
        },
    );

    let page = json!(["page", "run", "UTF-8", "header"]);
    assert_eq!(
        (
            fetches(&env, "paused"),
            pools(&env, "paused"),
            work_items(&run_file(&env, "paused")),
        ),
        (
            vec![
                page.clone(),
                page,
                json!(["book", "label", "windows-1252", "setting"]),
                json!(["author", "run", "UTF-8", "header"]),
            ],
            vec![String::new(), "book".to_owned()],
            vec![
                ("1".to_owned(), "succeeded".to_owned(), 1, None),
                ("2".to_owned(), "succeeded".to_owned(), 1, None),
                ("3".to_owned(), "succeeded".to_owned(), 1, None),
                (
                    "4".to_owned(),
                    "failed".to_owned(),
                    1,
                    Some("page_check_failed".to_owned())
                ),
            ],
        ),
    );
}

/// A resume records its start's declared Labels in the run file in place
/// of those the Run began with, and an engine that reopens the Run later
/// works each Label under that declaration (#2825).
///
/// Covers: operation:crawl.start, operation:run.resume, setting:fetch.encoding
#[test]
fn a_resume_records_its_labels_and_a_later_reopen_reads_them() {
    let env = TestEnv::new();
    let at_first = || {
        configure(&env, ONE_AT_A_TIME);
        default_dependencies(&env, RunStateOptions::default())
    };

    crawl(at_first, async |client| {
        paused_after_its_first_page(client, &env, paused_start(&json!({ "page": {} }))).await;
        client
            .call(
                "crawl.start",
                paused_start(&json!({ "page": {}, "book": book() })),
                &mut first_only,
            )
            .await;
        client
            .call("run.resume", json!({ "run_id": "paused" }), &mut first_only)
            .await;
        client
            .call("run.pause", json!({ "run_id": "paused" }), &mut first_only)
            .await;
    });
    let recorded = declared_labels(&env, "paused");
    crawl(at_first, async |client| {
        client
            .call(
                "run.resume",
                json!({ "run_id": "paused" }),
                &mut enqueue_a_book,
            )
            .await;
        client
            .call(
                "run.finish",
                json!({ "run_id": "paused" }),
                &mut enqueue_a_book,
            )
            .await;
    });

    let books: Vec<Value> = fetches(&env, "paused")
        .into_iter()
        .filter(|fetch| fetch[0] == "book")
        .collect();
    assert_eq!(
        (recorded, books),
        (
            vec![("book".to_owned(), book()), ("page".to_owned(), json!({}))],
            vec![json!(["book", "label", "windows-1252", "setting"])],
        )
    );
}

/// A resume that declares each Label as the Run began leaves their
/// settings as they were: the run file holds the same declaration, and
/// the pages still read in the Label's own encoding (#2825).
///
/// Covers: operation:crawl.start, setting:fetch.encoding
#[test]
fn a_resume_that_changes_no_label_leaves_its_settings_as_they_were() {
    let env = TestEnv::new();
    let page = json!({ "settings": { "fetch": { "encoding": "windows-1252" } } });
    let labels = json!({ "page": page });

    crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, RunStateOptions::default())
        },
        async |client| {
            paused_after_its_first_page(client, &env, paused_start(&labels)).await;
            client
                .call("crawl.start", paused_start(&labels), &mut first_only)
                .await;
            client
                .call("run.finish", json!({ "run_id": "paused" }), &mut answer_all)
                .await;
        },
    );

    let read = json!(["page", "run", "windows-1252", "setting"]);
    assert_eq!(
        (declared_labels(&env, "paused"), fetches(&env, "paused")),
        (vec![("page".to_owned(), page)], vec![read.clone(), read]),
    );
}

/// A resume its checks refuse, for a pending request's Label it no longer
/// declares or a Label's login pool it would send from a transport,
/// records none of its Labels: the run file keeps the Run's, and a resume
/// after it works under them (#2825).
///
/// Covers: operation:crawl.start, operation:run.resume
#[rstest]
/// Covers: error:label_unknown
#[case::label_unknown(json!({ "book": book() }), "label_unknown")]
/// Covers: error:setting_locked
#[case::setting_locked(
    json!({
        "page": {},
        "account": { "settings": {
            "session": { "source": { "kind": "saved_identity", "name": "shop" } },
            "fetch": { "http_from": "transport" },
        } },
    }),
    "setting_locked"
)]
fn a_refused_resume_records_none_of_its_labels(#[case] labels: Value, #[case] code: &str) {
    let env = TestEnv::new();

    let refused = crawl(
        || {
            configure(&env, ONE_AT_A_TIME);
            default_dependencies(&env, RunStateOptions::default())
        },
        async |client| {
            paused_after_its_first_page(client, &env, paused_start(&json!({ "page": {} }))).await;
            let refused = client
                .call("crawl.start", paused_start(&labels), &mut first_only)
                .await;
            client
                .call("run.resume", json!({ "run_id": "paused" }), &mut answer_all)
                .await;
            client
                .call("run.finish", json!({ "run_id": "paused" }), &mut answer_all)
                .await;
            refused
        },
    );

    assert_eq!(
        (
            refusal(&refused).0,
            declared_labels(&env, "paused"),
            work_items(&run_file(&env, "paused"))
                .into_iter()
                .map(|(_, status, _, _)| status)
                .collect::<Vec<_>>(),
        ),
        (
            json!(code),
            vec![("page".to_owned(), json!({}))],
            vec!["succeeded".to_owned(), "succeeded".to_owned()],
        ),
        "{refused}"
    );
}
