//! A crawl works only the Labels it declares (ADR 0093): a seed, a
//! sitemap rule, an enqueue, a seed added or a resume naming one it does
//! not declare fails with `label_unknown`, its details naming the Label
//! and those the crawl declares, and a resume that declares a new Label
//! takes it.

use goodfetch_engine::run_state::RunStateOptions;
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
