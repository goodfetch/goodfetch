//! Covers: story:5-119

use goodfetch_engine::response_limits::Limits;
use goodfetch_page::page_value::{ObservedResponse, ResponseObserver};
use goodfetch_protocol::page::PageId;
use rstest::rstest;
use serde_json::{Value, json};
use url::Url;

use super::{Call, Capture, infer};

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn call(at: &str, json: Value) -> Call {
    Call { url: url(at), json }
}

/// The URLs a plan replays for `document`, each answered as `answers`
/// says, the next asked for only after the one before was answered.
fn replayed(
    plan: &super::Plan,
    document: &str,
    answers: &dyn Fn(&str) -> Option<Value>,
) -> Vec<String> {
    let mut replay = plan.replay(&url(document));
    let mut asked = Vec::new();
    while let Some(next) = replay.next_call() {
        asked.push(next.to_string());
        assert!(asked.len() < 50, "{asked:?}");
        match answers(next.as_str()) {
            Some(answer) => replay.answered(&answer),
            None => replay.failed(),
        }
    }
    asked
}

/// Pages of a page-number listing: the number counts up in its query, and
/// the path follows the document's; an empty page ends it.
#[test]
fn a_page_number_plan_counts_its_page_up_from_the_first_call_and_follows_the_documents_path() {
    let calls = [
        call(
            "http://t.test/api/scroll/7?page=1",
            json!({ "items": ["a"] }),
        ),
        call(
            "http://t.test/api/scroll/7?page=2",
            json!({ "items": ["b"] }),
        ),
        call(
            "http://t.test/api/scroll/7?page=3",
            json!({ "items": ["c"] }),
        ),
        call(
            "http://t.test/api/scroll/7?page=1",
            json!({ "items": ["a"] }),
        ),
    ];
    let plan = infer(&url("http://t.test/scroll/7"), &calls).unwrap();

    let asked = replayed(&plan, "http://t.test/scroll/8", &|at| {
        Some(match at {
            "http://t.test/api/scroll/8?page=1" => json!({ "items": ["x"] }),
            "http://t.test/api/scroll/8?page=2" => json!({ "items": ["y"] }),
            _ => json!({ "items": [] }),
        })
    });

    assert_eq!(
        asked,
        [
            "http://t.test/api/scroll/8?page=1",
            "http://t.test/api/scroll/8?page=2",
            "http://t.test/api/scroll/8?page=3",
        ]
    );
}

/// A page number in the path counts up as one in the query does.
#[test]
fn a_page_number_in_the_path_counts_up_too() {
    let calls = [
        call("http://t.test/api/feed/1", json!({ "items": [1] })),
        call("http://t.test/api/feed/2", json!({ "items": [2] })),
    ];
    let plan = infer(&url("http://t.test/feed"), &calls).unwrap();

    let asked = replayed(&plan, "http://t.test/feed", &|at| {
        Some(if at.ends_with("/3") {
            json!({ "items": [] })
        } else {
            json!({ "items": [0] })
        })
    });

    assert_eq!(
        asked,
        [
            "http://t.test/api/feed/1",
            "http://t.test/api/feed/2",
            "http://t.test/api/feed/3",
        ]
    );
}

/// A cursor plan takes each call's cursor from where the call before's
/// answer held the one the browser sent, and ends where an answer holds
/// none.
#[test]
fn a_cursor_plan_takes_each_cursor_from_the_answer_before_it() {
    let calls = [
        call(
            "http://t.test/api/cursor/7",
            json!({ "items": ["a"], "page": { "next": "k1" } }),
        ),
        call(
            "http://t.test/api/cursor/7?after=k1",
            json!({ "items": ["b"], "page": { "next": "k2" } }),
        ),
        call(
            "http://t.test/api/cursor/7?after=k2",
            json!({ "items": ["c"], "page": { "next": null } }),
        ),
    ];
    let plan = infer(&url("http://t.test/cursor/7"), &calls).unwrap();

    let asked = replayed(&plan, "http://t.test/cursor/9", &|at| {
        Some(match at {
            "http://t.test/api/cursor/9" => json!({ "items": [1], "page": { "next": "z1" } }),
            "http://t.test/api/cursor/9?after=z1" => {
                json!({ "items": [2], "page": { "next": null } })
            }
            _ => return None,
        })
    });

    assert_eq!(
        asked,
        [
            "http://t.test/api/cursor/9",
            "http://t.test/api/cursor/9?after=z1",
        ]
    );
}

/// One call is replayed as it was, its path following the document's; a
/// failed call ends its series.
#[test]
fn a_single_call_is_replayed_once() {
    let calls = [call(
        "http://t.test/api/signed/7?token=abc",
        json!({ "items": ["a"] }),
    )];
    let plan = infer(&url("http://t.test/signed/7"), &calls).unwrap();

    let asked = replayed(&plan, "http://t.test/signed/7", &|_| None);

    assert_eq!(asked, ["http://t.test/api/signed/7?token=abc"]);
}

/// Calls of one endpoint that follow neither plan leave nothing to
/// replay, and so does a page that made no JSON call.
#[rstest::rstest]
#[case::no_call(Vec::new())]
#[case::unrelated_numbers(vec![
    call("http://t.test/api/items/1", json!({ "items": [1] })),
    call("http://t.test/api/items/5", json!({ "items": [5] })),
])]
#[case::two_values_change(vec![
    call("http://t.test/api/list?page=1&sort=a", json!({ "items": [1] })),
    call("http://t.test/api/list?page=2&sort=b", json!({ "items": [2] })),
])]
fn calls_that_follow_no_plan_give_none(#[case] calls: Vec<Call>) {
    assert_eq!(infer(&url("http://t.test/list"), &calls), None);
}

/// Calls captured in another order than they were made, as their answers'
/// bodies are read one beside another, are put back in order: by page
/// number, or by the chain of cursors.
#[test]
fn calls_captured_out_of_order_are_put_back_in_order() {
    let numbered = [
        call(
            "http://t.test/api/scroll/7?page=2",
            json!({ "items": ["b"] }),
        ),
        call(
            "http://t.test/api/scroll/7?page=1",
            json!({ "items": ["a"] }),
        ),
        call(
            "http://t.test/api/scroll/7?page=3",
            json!({ "items": ["c"] }),
        ),
    ];
    let cursored = [
        call(
            "http://t.test/api/cursor/7?after=k2",
            json!({ "page": { "next": null } }),
        ),
        call(
            "http://t.test/api/cursor/7?after=k1",
            json!({ "page": { "next": "k2" } }),
        ),
        call(
            "http://t.test/api/cursor/7",
            json!({ "page": { "next": "k1" } }),
        ),
    ];

    let pages = infer(&url("http://t.test/scroll/7"), &numbered).unwrap();
    let cursors = infer(&url("http://t.test/cursor/7"), &cursored).unwrap();

    assert_eq!(
        replayed(&pages, "http://t.test/scroll/7", &|at| {
            Some(json!({ "items": if at.ends_with("page=1") { vec![1] } else { vec![] } }))
        }),
        [
            "http://t.test/api/scroll/7?page=1",
            "http://t.test/api/scroll/7?page=2",
        ]
    );
    assert_eq!(
        replayed(&cursors, "http://t.test/cursor/7", &|_| None),
        ["http://t.test/api/cursor/7"]
    );
}

/// JSON `depth` levels deep, its arrays around one object.
fn nested(depth: usize) -> Vec<u8> {
    let arrays = depth - 1;
    format!(
        "{}{{\"next\":\"k1\"}}{}",
        "[".repeat(arrays),
        "]".repeat(arrays)
    )
    .into_bytes()
}

/// What a capture under `network.max_json_depth` `limit` keeps of a JSON
/// answer `depth` levels deep: `true` for its answer, `false` for the call
/// without it.
fn keeps_the_answer(depth: usize, limit: u64) -> bool {
    let capture = Capture::under(Limits {
        max_json_depth: limit,
        ..Limits::default()
    });
    let body = nested(depth);
    let page_id = PageId("page".to_owned());
    let headers = [("content-type".to_owned(), "application/json".to_owned())];
    capture.on_response(
        &ObservedResponse {
            page_id: &page_id,
            url: "http://t.test/api",
            body_length: body.len(),
            status: 200,
            headers: &headers,
            from_service_worker: false,
        },
        Some(&body),
    );
    let calls = capture.calls();
    assert_eq!(calls.len(), 1, "the call is captured either way");
    !calls[0].json.is_null()
}

/// `network.max_json_depth` decides which answers a capture keeps, up to the
/// ceiling, where a plan's walk of the answer still fits a stack: past 127
/// levels too, where the parser alone gave up, and a call past the depth is
/// kept without its answer, for its replay to refuse.
///
/// Covers: setting:network.max_json_depth
#[rstest]
#[case::within_the_default(128, 128, true)]
#[case::past_the_default(129, 128, false)]
#[case::under_a_raised_depth(129, 129, true)]
#[case::at_the_ceiling(1024, 1024, true)]
#[case::under_a_lowered_depth(4, 3, false)]
fn the_json_depth_decides_which_answers_a_capture_keeps(
    #[case] depth: usize,
    #[case] limit: u64,
    #[case] kept: bool,
) {
    assert_eq!(keeps_the_answer(depth, limit), kept);
}

/// A call captured without its answer, which the browser no longer held,
/// still tells its page number by its URL, and the same call made again
/// brings the answer a cursor plan reads.
#[test]
fn a_call_captured_without_its_answer_still_counts() {
    let numbered = [
        call(
            "http://t.test/api/scroll/7?page=1",
            json!({ "items": ["a"] }),
        ),
        call("http://t.test/api/scroll/7?page=2", Value::Null),
        call(
            "http://t.test/api/scroll/7?page=3",
            json!({ "items": ["c"] }),
        ),
    ];
    let cursored = [
        call("http://t.test/api/cursor/7", Value::Null),
        call("http://t.test/api/cursor/7?after=k1", Value::Null),
        call("http://t.test/api/cursor/7", json!({ "next": "k1" })),
    ];

    assert!(infer(&url("http://t.test/scroll/7"), &numbered).is_some());
    let plan = infer(&url("http://t.test/cursor/7"), &cursored).unwrap();
    let asked = replayed(&plan, "http://t.test/cursor/7", &|at| {
        Some(match at {
            "http://t.test/api/cursor/7" => json!({ "next": "k9" }),
            _ => json!({ "next": null }),
        })
    });
    assert_eq!(
        asked,
        [
            "http://t.test/api/cursor/7",
            "http://t.test/api/cursor/7?after=k9",
        ]
    );
}
