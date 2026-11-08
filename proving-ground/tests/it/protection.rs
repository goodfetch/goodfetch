//! The `protection` family: a site whose every page answers a client with a
//! script-run Challenge that a browser passes and a plain HTTP client never
//! does (the tier-move site of Foxhound's spec #600), and a local CAPTCHA.
//! The oracle records which tier cleared each page and the fingerprint of
//! every request, and judges a result from that record.
//!
//! The browser here is a plain HTTP client that does what the Challenge's
//! script does: it reads the page's nonce, sets the clearance cookie the
//! script computes, loads the page again and posts what the page's script
//! posts once it ran.
//! Covers: story:2-33

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::clients;
use crate::proving_ground::{Answer, ProvingGround, submission};

const SEED: u64 = 167;

/// The first capture of `pattern` in `text`, if any.
fn find(text: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .expect("a valid pattern")
        .captures(text)
        .map(|captures| captures[1].to_owned())
}

/// The first capture of `pattern` in `text`.
fn capture(text: &str, pattern: &str) -> String {
    find(text, pattern).unwrap_or_else(|| panic!("{pattern} in {text}"))
}

/// The task `id`'s start URL.
fn start_url(proving_ground: &ProvingGround, id: &str) -> String {
    proving_ground.tasks()["tasks"]
        .as_array()
        .expect("`tasks` is an array")
        .iter()
        .find(|task| task["id"] == id)
        .and_then(|task| task["start_url"].as_str())
        .unwrap_or_else(|| panic!("the task list names {id}"))
        .to_owned()
}

/// The origin of a URL: its scheme, host and port.
fn origin(url: &str) -> String {
    let rest = url.strip_prefix("http://").expect("an http URL");
    format!("http://{}", &rest[..rest.find('/').unwrap_or(rest.len())])
}

/// The path of a URL.
fn path(url: &str) -> String {
    let rest = url.strip_prefix("http://").expect("an http URL");
    rest[rest.find('/').unwrap_or(rest.len())..].to_owned()
}

/// The clearance the Challenge's script computes from its nonce: each
/// character folded into `7` as `hash * 31 + code`, kept to 32 bits, in
/// lowercase hex.
fn clearance(nonce: &str) -> String {
    let hash = nonce.bytes().fold(7_u64, |hash, byte| {
        (hash * 31 + u64::from(byte)) & 0xFFFF_FFFF
    });
    format!("{hash:x}")
}

/// A browser of the protection site: it passes each Challenge as its script
/// does, keeps the clearance cookie, and posts that each page's script ran.
struct Browser<'a> {
    proving_ground: &'a ProvingGround,
    cookie: Option<String>,
}

impl<'a> Browser<'a> {
    fn new(proving_ground: &'a ProvingGround) -> Self {
        Self {
            proving_ground,
            cookie: None,
        }
    }

    fn get(&self, url: &str) -> Answer {
        match &self.cookie {
            Some(cookie) => self.proving_ground.get_with_cookie(url, Some(cookie)),
            None => self.proving_ground.get(url),
        }
    }

    /// Opens a page, passing its Challenge if it shows one, and runs its
    /// script.
    fn open(&mut self, url: &str) -> String {
        let mut page = self.get(url);
        if let Some(nonce) = find(&page.body, r#"data-nonce="([^"]+)""#) {
            self.cookie = Some(format!("clearance={}", clearance(&nonce)));
            page = self.get(url);
        }
        assert_eq!(page.status, 200, "{}", page.body);
        let ran = self.proving_ground.post_json(
            &format!("{}/reports", origin(url)),
            &json!({ "page": path(url), "check": "ran", "observed": "content" }),
        );
        assert_eq!(ran.status, 204, "{}", ran.body);
        page.body
    }
}

/// The article links of the site's index page.
fn article_links(origin: &str, index: &str) -> Vec<String> {
    Regex::new(r#"href="(/articles/[0-9]+)""#)
        .expect("a valid pattern")
        .captures_iter(index)
        .map(|link| format!("{origin}{}", &link[1]))
        .collect()
}

/// The Record an article page shows.
fn read_article(page: &str) -> Value {
    json!({
        "id": capture(page, r#"<article class="article" data-id="([^"]+)""#),
        "title": capture(page, r#"<h1 class="title">([^<]+)</h1>"#),
    })
}

/// Each page's tier in the oracle's answer, by page.
fn tiers(answer: &Value) -> Vec<(String, Value)> {
    answer["pages"]
        .as_array()
        .expect("`pages` is an array")
        .iter()
        .map(|page| {
            (
                page["page"].as_str().expect("a page").to_owned(),
                page["cleared_by"].clone(),
            )
        })
        .collect()
}

/// A crawl in the browser of the index and every article it links: the
/// article URLs and their Records.
fn browser_crawl(proving_ground: &ProvingGround) -> (Vec<String>, Vec<Value>) {
    let url = start_url(proving_ground, "protection/1");
    let mut browser = Browser::new(proving_ground);
    let index = browser.open(&url);
    let links = article_links(&origin(&url), &index);
    let records = links
        .iter()
        .map(|link| read_article(&browser.open(link)))
        .collect();
    (links, records)
}

#[test]
fn a_plain_http_client_meets_the_challenge_on_every_page() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/1");

    let page = proving_ground.get(&url);

    assert_eq!(page.status, 403, "{}", page.body);
    assert!(
        find(&page.body, r#"data-nonce="([^"]+)""#).is_some(),
        "{}",
        page.body
    );
}

#[test]
fn a_crawl_the_browser_cleared_is_verified_with_each_page_cleared_by_the_browser() {
    let proving_ground = ProvingGround::start(SEED);
    let (_, records) = browser_crawl(&proving_ground);

    let answer = proving_ground.judge(&submission("protection/1", "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
    assert_eq!(
        tiers(&answer),
        [
            ("/articles".to_owned(), json!("browser")),
            ("/articles/1".to_owned(), json!("browser")),
            ("/articles/2".to_owned(), json!("browser")),
            ("/articles/3".to_owned(), json!("browser")),
            ("/articles/4".to_owned(), json!("browser")),
            ("/articles/5".to_owned(), json!("browser")),
        ]
    );
}

#[test]
fn a_crawl_with_a_page_no_tier_cleared_is_incomplete_naming_the_page() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/1");
    let mut browser = Browser::new(&proving_ground);
    let index = browser.open(&url);
    let links = article_links(&origin(&url), &index);
    let (last, cleared) = links.split_last().expect("the index links articles");
    let records: Vec<Value> = cleared
        .iter()
        .map(|link| read_article(&browser.open(link)))
        .collect();
    let challenged = proving_ground.get(last);

    let answer = proving_ground.judge(&submission("protection/1", "failure", &records));

    assert_eq!(challenged.status, 403);
    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!(["T-0005"]), "{answer}");
    assert_eq!(tiers(&answer)[5], ("/articles/5".to_owned(), Value::Null));
}

#[test]
fn a_challenge_page_committed_as_records_is_a_false_success_naming_each_uncleared_page() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/1");
    let records: Vec<Value> = (1..=5)
        .map(|number| {
            let page = proving_ground.get(&format!("{}/articles/{number}", origin(&url)));
            json!({
                "id": format!("T-{number:04}"),
                "title": capture(&page.body, "<title>([^<]+)</title>"),
            })
        })
        .collect();

    let answer = proving_ground.judge(&submission("protection/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["findings"][0],
        json!({ "check": "uncleared_page", "at": "/articles/1", "observed": "T-0001" })
    );
    assert_eq!(answer["findings"].as_array().map(Vec::len), Some(5));
    assert!(
        tiers(&answer).iter().all(|(_, tier)| tier.is_null()),
        "{answer}"
    );
}

#[test]
fn a_page_fetched_with_the_browsers_clearance_but_no_script_was_cleared_over_http() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/1");
    let nonce = capture(&proving_ground.get(&url).body, r#"data-nonce="([^"]+)""#);
    let cookie = format!("clearance={}", clearance(&nonce));

    let page = proving_ground.get_with_cookie(&url, Some(&cookie));
    let answer = proving_ground.judge(&submission("protection/1", "failure", &[]));

    assert_eq!(page.status, 200, "{}", page.body);
    assert_eq!(tiers(&answer)[0], ("/articles".to_owned(), json!("http")));
}

#[test]
fn the_oracle_records_the_fingerprint_of_every_request() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/1");
    let host = capture(&url, r"http://([^:/]+)");
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let request = format!(
        "GET /articles HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Label/1.0\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    );
    clients::raw_tls(proving_ground.address(), &host, config, request.as_bytes())
        .expect("the handshake succeeds");

    let answer = proving_ground.judge(&submission("protection/1", "failure", &[]));

    let seen = &answer["requests"][0];
    assert_eq!(seen["target"], "/articles", "{answer}");
    assert_eq!(seen["status"], 403, "{answer}");
    assert_eq!(seen["fingerprint"]["protocol"], "http/1.1", "{answer}");
    assert_eq!(seen["fingerprint"]["user_agent"], "Label/1.0", "{answer}");
    assert_eq!(
        seen["fingerprint"]["header_order"],
        json!(["host", "user-agent", "accept", "connection"]),
        "{answer}"
    );
    assert!(
        seen["fingerprint"]["ja4"]
            .as_str()
            .is_some_and(|ja4| ja4.starts_with("t13d")),
        "{answer}"
    );
}

/// The local CAPTCHA's code, as the characters its image draws in order.
fn captcha_code(page: &str) -> String {
    Regex::new(r#"<text [^>]*class="glyph"[^>]*>([^<])</text>"#)
        .expect("a valid pattern")
        .captures_iter(page)
        .map(|glyph| glyph[1].to_owned())
        .collect()
}

#[test]
fn a_tool_that_solves_the_captcha_reads_the_article_behind_it_and_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/2");
    let code = captcha_code(&proving_ground.get(&url).body);
    let solved = proving_ground.post_form(&url, &[("answer", &code)], None);
    let cookie = solved
        .cookie("captcha")
        .expect("a solved CAPTCHA sets its cookie");
    let record = read_article(&proving_ground.get_with_cookie(&url, Some(&cookie)).body);

    let answer = proving_ground.judge(&submission("protection/2", "success", &[record]));

    assert_eq!(solved.status, 200, "{}", solved.body);
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[rstest]
#[case::empty("")]
#[case::wrong("ZZZZZ")]
fn a_wrong_captcha_answer_is_refused_without_a_clearance(#[case] code: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/2");

    let refused = proving_ground.post_form(&url, &[("answer", code)], None);

    assert_eq!(refused.status, 403, "{}", refused.body);
    assert_eq!(refused.cookie("captcha"), None);
}

#[test]
fn the_captcha_page_committed_as_a_record_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let url = start_url(&proving_ground, "protection/2");
    let page = proving_ground.get(&url).body;
    let record = json!({ "id": "captcha", "title": capture(&page, "<title>([^<]+)</title>") });

    let answer = proving_ground.judge(&submission("protection/2", "success", &[record]));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(
        answer["missing"].as_array().map(Vec::len),
        Some(1),
        "{answer}"
    );
}
