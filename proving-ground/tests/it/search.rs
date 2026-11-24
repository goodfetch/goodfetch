//! The `search` family: a search results page and a SearXNG-compatible API,
//! whose results change with the time step.
//!
//! The known-correct results come from searching at each step of the task
//! over plain HTTP and keeping each result it shows once.
//! Covers: story:2-33

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{Answer, ProvingGround, submission};
use crate::reading::{start_url, task};

const SEED: u64 = 1540;
const PAGE_TASK: &str = "search/1";
const API_TASK: &str = "search/2";

/// The records a results page shows.
fn page_results(answer: &Answer) -> Vec<Value> {
    assert_eq!(answer.status, 200, "{}", answer.body);
    Regex::new(
        r#"data-id="([^"]+)">\n<h3><a href="([^"]+)">([^<]+)</a></h3>\n<p class="content">([^<]+)</p>"#,
    )
    .expect("a valid pattern")
    .captures_iter(&answer.body)
    .map(|found| json!({ "id": &found[1], "url": &found[2], "title": &found[3], "snippet": &found[4] }))
    .collect()
}

/// The records a JSON answer holds.
fn api_results(answer: &Answer) -> Vec<Value> {
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()["results"]
        .as_array()
        .expect("`results` is an array")
        .iter()
        .map(|result| {
            let url = result["url"].as_str().expect("a URL");
            json!({
                "id": url.rsplit('/').next(),
                "url": url,
                "title": result["title"],
                "snippet": result["content"],
            })
        })
        .collect()
}

/// How a program reads a task's search.
type Read = fn(&Answer) -> Vec<Value>;

/// What a search shows at each of `steps`, each result once.
fn watched(
    proving_ground: &ProvingGround,
    id: &str,
    read: Read,
    steps: impl IntoIterator<Item = u64>,
) -> Vec<Value> {
    let task = task(proving_ground, id);
    let mut records = Vec::new();
    for step in steps {
        proving_ground.set_time_step(&json!(step));
        for record in read(&proving_ground.get(start_url(&task))) {
            if !records.contains(&record) {
                records.push(record);
            }
        }
    }
    records
}

fn steps(proving_ground: &ProvingGround, id: &str) -> u64 {
    task(proving_ground, id)["time_steps"]
        .as_u64()
        .expect("a count of time steps")
}

/// The query the API task searches for.
fn query(proving_ground: &ProvingGround) -> String {
    let answer = proving_ground.get(start_url(&task(proving_ground, API_TASK)));
    answer.json()["query"].as_str().expect("a query").to_owned()
}

fn search(proving_ground: &ProvingGround, query: &str) -> Answer {
    proving_ground.get(&proving_ground.url("search.test", &format!("/search?{query}")))
}

#[rstest]
#[case::the_page(PAGE_TASK, page_results)]
#[case::the_api(API_TASK, api_results)]
fn the_results_of_every_step_are_verified(#[case] id: &str, #[case] read: Read) {
    let proving_ground = ProvingGround::start(SEED);
    let steps = steps(&proving_ground, id);
    let records = watched(&proving_ground, id, read, 0..steps);

    let answer = proving_ground.judge(&submission(id, "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::the_page_once(PAGE_TASK, page_results, &[0])]
#[case::the_api_once(API_TASK, api_results, &[0])]
#[case::the_page_until_it_changes_again(PAGE_TASK, page_results, &[0, 1, 2])]
#[case::the_api_after_its_last_change(API_TASK, api_results, &[3])]
fn stale_results_are_refuted(#[case] id: &str, #[case] read: Read, #[case] steps: &[u64]) {
    let proving_ground = ProvingGround::start(SEED);
    let records = watched(&proving_ground, id, read, steps.iter().copied());

    let answer = proving_ground.judge(&submission(id, "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
}

#[test]
fn the_results_of_a_step_are_the_same_whenever_they_are_asked() {
    let first = ProvingGround::start(SEED);
    let earlier = watched(&first, API_TASK, api_results, [2]);
    let later = watched(&first, API_TASK, api_results, [3]);

    assert_ne!(earlier, later);
    assert_eq!(watched(&first, API_TASK, api_results, [2]), earlier);
    assert_eq!(watched(&first, API_TASK, api_results, [1]), earlier);
    let again = ProvingGround::start(SEED);
    assert_eq!(watched(&again, API_TASK, api_results, [3]), later);
    let other = ProvingGround::start(SEED + 1);
    assert_ne!(watched(&other, API_TASK, api_results, [3]), later);
}

#[test]
fn the_api_answers_the_fields_a_search_provider_reads() {
    let proving_ground = ProvingGround::start(SEED);
    let query = query(&proving_ground);

    let answer = proving_ground.get(start_url(&task(&proving_ground, API_TASK)));

    assert_eq!(answer.header("content-type"), Some("application/json"));
    let answer = answer.json();
    assert_eq!(answer["query"], query);
    for list in [
        "answers",
        "corrections",
        "infoboxes",
        "suggestions",
        "unresponsive_engines",
    ] {
        assert_eq!(answer[list], json!([]), "{list}");
    }
    let results = answer["results"].as_array().expect("`results` is an array");
    assert_eq!(results.len(), 3);
    for result in results {
        for field in ["url", "title", "content", "engine", "category"] {
            assert!(
                result[field].as_str().is_some_and(|text| !text.is_empty()),
                "{field} of {result}"
            );
        }
        assert_eq!(result["engines"], json!([result["engine"]]));
        assert!(result["score"].is_f64(), "{result}");
    }
}

#[rstest]
#[case::a_posted_form(&[("format", "json")], "")]
#[case::a_form_over_the_query(&[("format", "json")], "?q=nothing&format=html")]
#[case::every_parameter(
    &[
        ("format", "json"),
        ("pageno", "1"),
        ("language", "en-GB"),
        ("safesearch", "0"),
        ("time_range", "month"),
        ("categories", "general"),
        ("engines", "articles"),
    ],
    ""
)]
fn the_api_answers_a_posted_form(#[case] form: &[(&str, &str)], #[case] target: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let query = query(&proving_ground);
    let mut form = form.to_vec();
    form.push(("q", &query));
    let expected = proving_ground.get(start_url(&task(&proving_ground, API_TASK)));

    let answer = proving_ground.post_form(
        &proving_ground.url("search.test", &format!("/search{target}")),
        &form,
        None,
    );

    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json(), expected.json());
}

#[rstest]
#[case::the_next_page("q=tide+mill&pageno=2")]
#[case::other_words("q=no+such+thing")]
fn a_search_that_finds_nothing_answers_no_result(#[case] query: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = search(&proving_ground, &format!("{query}&format=json"));

    assert_eq!(api_results(&answer), Vec::<Value>::new());
    let page = search(&proving_ground, query);
    assert_eq!(page_results(&page), Vec::<Value>::new());
    assert!(page.body.contains("No results were found"), "{}", page.body);
}

#[rstest]
#[case::no_query("", "No query")]
#[case::an_empty_query("q=", "No query")]
#[case::page_zero("q=a&pageno=0", "Invalid value 0 for parameter pageno")]
#[case::a_negative_page("q=a&pageno=-1", "Invalid value -1 for parameter pageno")]
#[case::no_page("q=a&pageno=", "Empty pageno parameter")]
#[case::a_word_for_a_page("q=a&pageno=two", "Invalid value two for parameter pageno")]
#[case::a_language("q=a&language=english", "Invalid value english for parameter language")]
#[case::a_region("q=a&language=en-GBR", "Invalid value en-GBR for parameter language")]
#[case::safe_search("q=a&safesearch=3", "Invalid value 3 for parameter safesearch")]
#[case::a_time_range("q=a&time_range=hour", "Invalid value hour for parameter time_range")]
fn the_api_refuses_a_search_searxng_refuses(#[case] query: &str, #[case] message: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = search(&proving_ground, &format!("{query}&format=json"));

    assert_eq!(answer.status, 400, "{}", answer.body);
    assert_eq!(answer.json(), json!({ "error": message }));
}

#[rstest]
#[case::csv("csv")]
#[case::rss("rss")]
fn a_format_the_instance_does_not_allow_is_refused(#[case] format: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = search(&proving_ground, &format!("q=tide+mill&format={format}"));

    assert_eq!(answer.status, 403, "{}", answer.body);
    assert_eq!(page_results_of(&answer), 0);
}

fn page_results_of(answer: &Answer) -> usize {
    answer.body.matches("data-id").count()
}

#[rstest]
#[case::no_query("", 200)]
#[case::an_unknown_format("q=tide+mill&format=xml", 200)]
#[case::a_bad_page("q=tide+mill&pageno=0", 400)]
fn the_page_answers_as_searxng_does(#[case] query: &str, #[case] status: u16) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = search(&proving_ground, query);

    assert_eq!(answer.status, status, "{}", answer.body);
    assert_eq!(
        answer.header("content-type"),
        Some("text/html; charset=utf-8")
    );
}

#[test]
fn a_query_is_never_read_as_the_page_s_own_html() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = search(&proving_ground, "q=%22%3E%3Cb%3E%26");

    assert!(
        answer.body.contains("value=\"&quot;&gt;&lt;b&gt;&amp;\""),
        "{}",
        answer.body
    );
}

#[rstest]
#[case::another_path("/autocompleter?q=tide", 404)]
#[case::the_settings("/config", 404)]
fn every_other_call_is_refused(#[case] path: &str, #[case] status: u16) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.get(&proving_ground.url("search.test", path));

    assert_eq!(answer.status, status, "{}", answer.body);
}

#[test]
fn the_index_of_another_method_is_refused() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.post_json(&proving_ground.url("search.test", "/"), &json!({}));

    assert_eq!(answer.status, 405, "{}", answer.body);
}
