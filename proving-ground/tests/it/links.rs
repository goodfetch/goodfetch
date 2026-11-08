//! The `links` family (pitfalls U08 and U14): a shelf whose relative links
//! resolve against its `<base href>`, one reached through a redirect whose
//! relative links resolve against the URL it moved to, and rooms linked
//! only from scripts (an `onclick` navigation, a popup and `pushState`). The
//! Proving ground catches a crawl that resolved against the wrong base from
//! the requests it made, and the oracle names the pages a crawl that read
//! only the HTML's links never reached.
//! Covers: story:2-33

use serde_json::{Value, json};

use crate::crawler::{Base, Crawler, Fetched, field, finding_kinds, links, script_links};
use crate::proving_ground::{ProvingGround, submission};

const SEED: u64 = 1538;

fn records(pages: &[Fetched]) -> Vec<Value> {
    pages
        .iter()
        .filter_map(|page| {
            let body = &page.answer.body;
            Some(json!({
                "id": field(body, r#"<article class="entry" data-id="([^"]+)""#)?,
                "title": field(body, r#"<h1 class="title">([^<]+)</h1>"#)?,
            }))
        })
        .collect()
}

fn start_url(proving_ground: &ProvingGround, task: &str) -> String {
    proving_ground.task(task)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

#[test]
fn a_crawl_that_resolves_against_the_base_element_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground, "links/1"));

    let answer = proving_ground.judge(&submission("links/1", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(answer["findings"], json!([]));
    assert_eq!(records(&pages).len(), 8, "{answer}");
}

#[test]
fn a_crawl_that_ignores_the_base_element_is_a_false_success_with_wrong_base_findings() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.base = Base::Requested;
    let pages = crawler.crawl(&start_url(&proving_ground, "links/1"));

    let answer = proving_ground.judge(&submission("links/1", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"].as_array().unwrap().len(), 8, "{answer}");
    assert_eq!(finding_kinds(&answer), ["wrong_base"], "{answer}");
}

#[test]
fn a_crawl_that_resolves_against_the_url_a_redirect_named_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "links/2");
    assert_eq!(proving_ground.get(&start).status, 301);
    let pages = Crawler::new(&proving_ground).crawl(&start);

    let answer = proving_ground.judge(&submission("links/2", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(records(&pages).len(), 8, "{answer}");
}

#[test]
fn a_crawl_that_resolves_against_the_url_before_the_redirect_is_caught() {
    let proving_ground = ProvingGround::start(SEED);
    let mut crawler = Crawler::new(&proving_ground);
    crawler.base = Base::Requested;
    let pages = crawler.crawl(&start_url(&proving_ground, "links/2"));

    let answer = proving_ground.judge(&submission("links/2", "needs-review", &records(&pages)));

    assert_eq!(answer["outcome"], "failed", "{answer}");
    assert_eq!(finding_kinds(&answer), ["wrong_base"], "{answer}");
}

#[test]
fn a_crawl_that_follows_the_script_only_links_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let pages = Crawler::new(&proving_ground).crawl(&start_url(&proving_ground, "links/3"));

    let answer = proving_ground.judge(&submission("links/3", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(records(&pages).len(), 12, "{answer}");
}

#[test]
fn a_crawl_that_reads_only_the_html_links_is_a_false_success_naming_the_rooms_it_missed() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "links/3");
    let mut crawler = Crawler::new(&proving_ground);
    crawler.follow_scripts = false;
    let pages = crawler.crawl(&start);

    let answer = proving_ground.judge(&submission("links/3", "success", &records(&pages)));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let scripted = script_links(&proving_ground.get(&start).body);
    assert_eq!(scripted.len(), 8, "{scripted:?}");
    let missing: Vec<String> = answer["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| format!("/rooms/{}", id.as_str().unwrap()))
        .collect();
    assert_eq!(missing, scripted, "{answer}");
}

#[test]
fn a_script_only_link_is_neither_an_anchor_nor_a_path_in_the_page() {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground
        .get(&start_url(&proving_ground, "links/3"))
        .body;

    for path in script_links(&page) {
        assert!(!page.contains(&path), "{path} is spelled out in the page");
        assert!(!links(&page, true).contains(&path), "{path}");
    }
    for call in ["location.assign(", "window.open(", "history.pushState("] {
        assert!(page.contains(call), "the page has no {call}");
    }
}

#[test]
fn a_crawl_of_the_script_only_rooms_reaches_them_only_through_its_own_requests() {
    let proving_ground = ProvingGround::start(SEED);
    let start = start_url(&proving_ground, "links/3");
    let mut crawler = Crawler::new(&proving_ground);
    crawler.follow_scripts = false;
    crawler.crawl(&start);

    let observations = proving_ground.observations().to_string();

    assert!(observations.contains("\"/rooms/R-01\""), "{observations}");
    for path in script_links(&proving_ground.get(&start).body) {
        assert!(!observations.contains(&format!("\"{path}\"")), "{path}");
    }
}

/// `links.test/answers/` links one page twice, an anchor with no `href`, a
/// mail address, an off-site page, a URL that does not parse and two more
/// pages, one carrying its ISBN: an `enqueueLinks` call's every answer.
#[test]
fn the_answers_page_links_every_answer_an_enqueue_links_call_gives() {
    let proving_ground = ProvingGround::start(SEED);
    let index = proving_ground.get(&proving_ground.url("links.test", "/answers/"));
    let pages: Vec<u16> = (1..=3)
        .map(|page| {
            proving_ground
                .get(&proving_ground.url("links.test", &format!("/answers/{page}")))
                .status
        })
        .collect();

    assert_eq!(index.status, 200);
    assert!(
        index.body.contains(
            "<ul class=\"links\">\n<li><a href=\"/answers/1\">One</a></li>\n<li><a href=\"/answers/1#top\">One again</a></li>\n<li><a name=\"here\">No link</a></li>\n<li><a href=\"mailto:shop@links.test\">Mail</a></li>\n<li><a href=\"http://elsewhere.test/\">Elsewhere</a></li>\n<li><a href=\"http://[::1/\">Broken</a></li>\n<li><a href=\"2\">Two</a></li>\n<li><a href=\"3\" data-isbn=\"978-3\">Three</a></li>\n</ul>"
        ),
        "{}",
        index.body
    );
    assert_eq!(pages, [200, 200, 200]);
}

/// `links.test/shift-jis/` is written in `Shift_JIS`, labelled so, and links
/// `search?q=日本&k=한`, 한 written as a character reference since
/// `Shift_JIS` has no 한; its search page answers any query.
#[test]
fn the_shift_jis_page_links_a_query_in_shift_jis() {
    let proving_ground = ProvingGround::start(SEED);
    let page = proving_ground.get_raw(&proving_ground.url("links.test", "/shift-jis/"));
    let search = proving_ground.get(&proving_ground.url(
        "links.test",
        "/shift-jis/search?q=%93%FA%96{&k=%26%2354620%3B",
    ));

    let (text, _, malformed) = encoding_rs::SHIFT_JIS.decode(&page.body);
    assert_eq!(
        (
            page.status,
            page.content_type.as_deref(),
            malformed,
            text.contains("<a href=\"search?q=日本&amp;k=&#54620;\">日本</a>"),
            search.status
        ),
        (200, Some("text/html; charset=Shift_JIS"), false, true, 200),
        "{text}"
    );
}
