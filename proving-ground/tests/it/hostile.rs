//! ADR 0067's hostile input beside SSRF: the `redirects`, `headers` and
//! `downloads` families, each page served with its expected result, which
//! names what a correct tool refuses, with which typed error where one is
//! decided, or keeps.
//!
//! A refusing tool is played by a submission that reports the failure the
//! expected result names; a non-refusing tool by one that follows the page
//! and reports what it reached, or keeps a file under the name the page
//! suggests.
//! Covers: story:2-83

use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 312;

fn start_url(proving_ground: &ProvingGround, id: &str) -> String {
    proving_ground.task(id)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

/// A failure that reports the typed error `error`.
fn refusal(task: &str, error: &Value) -> Value {
    json!({ "task": task, "status": "failure", "error": error })
}

/// The origin of a URL on one of the Proving ground's host names.
fn origin(url: &str) -> &str {
    let rest = url.strip_prefix("http://").expect("an http URL");
    &url[..7 + rest.find('/').unwrap_or(rest.len())]
}

/// Follows same-host `302` redirects from `url`, as a tool with no limit
/// does, and returns the page it ends on and how many it followed.
fn follow(proving_ground: &ProvingGround, url: &str) -> (String, u32) {
    let mut url = url.to_owned();
    let mut followed = 0;
    loop {
        let answer = proving_ground.get(&url);
        let Some(location) = answer.header("location").map(str::to_owned) else {
            return (answer.body, followed);
        };
        assert!(followed < 100, "the redirects end");
        url = format!("{}{location}", origin(&url));
        followed += 1;
    }
}

/// The Record a kept page shows.
fn read_record(page: &str) -> Value {
    let id = page
        .split("data-id=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the page shows a Record's id");
    let word = page
        .split("<h1 class=\"word\">")
        .nth(1)
        .and_then(|rest| rest.split('<').next())
        .expect("the page shows a Record's word");
    json!({ "id": id, "word": word })
}

#[rstest]
#[case::a_301_to_loopback("redirects/1", 301, "http://127.0.0.1/", "private_address")]
#[case::a_307_to_a_mapped_address(
    "redirects/10",
    307,
    "http://[::ffff:127.0.0.1]/",
    "private_address"
)]
#[case::a_302_to_a_file("redirects/11", 302, "file:///etc/passwd", "scheme")]
#[case::a_302_to_gopher("redirects/15", 302, "gopher://127.0.0.1:6379/_INFO", "scheme")]
fn a_redirect_case_answers_as_its_stand_in_and_its_typed_refusal_is_verified(
    #[case] id: &str,
    #[case] status: u16,
    #[case] location: &str,
    #[case] rule: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground.get(&start_url(&proving_ground, id));

    let answer = proving_ground
        .judge(&refusal(
            id,
            &json!({ "code": "egress_refused", "rule": rule }),
        ))
        .json();

    assert_eq!(page.status, status);
    assert_eq!(page.header("location"), Some(location));
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_redirect_case_refused_with_another_typed_error_fails_and_names_both() {
    let proving_ground = ProvingGround::start(SEED);
    let reported = json!({ "code": "egress_refused", "rule": "scheme" });

    let answer = proving_ground
        .judge(&refusal("redirects/1", &reported))
        .json();

    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(
        answer["wrong_refusal"],
        json!({
            "expected": { "code": "egress_refused", "rule": "private_address" },
            "submitted": reported,
        })
    );
}

#[rstest]
#[case::claimed_a_success("success", "false_success")]
#[case::reported_needs_review("needs-review", "failed")]
fn a_tool_that_does_not_refuse_a_redirect_case_is_judged_a_failure(
    #[case] status: &str,
    #[case] outcome: &str,
) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground
        .judge(&submission("redirects/6", status, &[]))
        .json();

    assert_eq!(answer["outcome"], outcome, "{answer}");
}

#[rstest]
#[case::a_refresh_header("redirects/6")]
#[case::a_meta_refresh("redirects/7")]
#[case::a_script("redirects/8")]
fn a_redirect_without_a_status_names_its_target_in_the_page(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground.get(&start_url(&proving_ground, id));

    assert_eq!(page.status, 200);
    let named = page.header("refresh").unwrap_or_default().to_owned() + &page.body;
    assert!(named.contains("http://127.0.0.1/"), "{named}");
}

#[test]
fn a_tool_that_refuses_the_redirect_to_the_proving_grounds_own_loopback_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground.get(&start_url(&proving_ground, "redirects/16"));
    let error = json!({ "code": "egress_refused", "rule": "private_address" });

    let answer = proving_ground
        .judge(&refusal("redirects/16", &error))
        .json();

    assert_eq!(page.status, 302);
    assert_eq!(
        page.header("location"),
        Some(
            format!(
                "http://127.0.0.1:{}/redirects/landing",
                proving_ground.port()
            )
            .as_str()
        )
    );
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_tool_that_follows_the_redirect_to_its_own_loopback_is_recorded_reaching_it() {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground.get(&start_url(&proving_ground, "redirects/16"));
    let landing = page.header("location").expect("a redirect").to_owned();
    let reached = proving_ground.control(
        landing
            .strip_prefix(&format!("http://127.0.0.1:{}", proving_ground.port()))
            .expect("a landing on the Proving ground's own address"),
    );
    let error = json!({ "code": "egress_refused", "rule": "private_address" });

    let answer = proving_ground
        .judge(&refusal("redirects/16", &error))
        .json();

    assert_eq!(reached.status, 200);
    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(answer["findings"][0]["check"], "private_address_reached");
}

#[test]
fn a_chain_of_20_redirects_is_followed_to_its_record() {
    let proving_ground = ProvingGround::start(SEED);
    let (page, followed) = follow(&proving_ground, &start_url(&proving_ground, "redirects/17"));

    let answer = proving_ground
        .judge(&submission(
            "redirects/17",
            "success",
            &[read_record(&page)],
        ))
        .json();

    assert_eq!(followed, 20);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::a_chain_of_21("redirects/18")]
#[case::a_loop("redirects/19")]
fn too_many_redirects_is_the_only_right_answer_past_20(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let refused = proving_ground
        .judge(&refusal(id, &json!({ "code": "too_many_redirects" })))
        .json();
    let claimed = proving_ground.judge(&submission(id, "success", &[])).json();

    assert_eq!(refused["outcome"], "verified", "{refused}");
    assert_eq!(claimed["outcome"], "false_success", "{claimed}");
}

#[test]
fn a_tool_that_follows_a_chain_of_21_and_keeps_its_record_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let (page, followed) = follow(&proving_ground, &start_url(&proving_ground, "redirects/18"));

    let answer = proving_ground
        .judge(&submission(
            "redirects/18",
            "success",
            &[read_record(&page)],
        ))
        .json();

    assert_eq!(followed, 21);
    assert_eq!(answer["outcome"], "false_success", "{answer}");
}
