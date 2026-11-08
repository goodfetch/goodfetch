//! The `languages` family (ADR 0073): pages in scripts written right to
//! left, without spaces, with conjuncts, combining marks, emoji sequences
//! and astral characters, the Unicode record, a page at a non-ASCII path and
//! query, and one on an IDN host name. The oracle holds each text in UTF-8,
//! in the normalization form served, and names the character that differs.
//! Covers: story:2-33, story:2-90

use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};
use crate::reading::{every, first, origin, start_url, task};

const SEED: u64 = 165;

/// The IDN host name `bücher.languages.test`, as a URL carries it.
const IDN_HOST: &str = "xn--bcher-kva.languages.test";

/// A URL with each byte of its non-ASCII characters percent-encoded, as a
/// browser sends a link written in Unicode.
fn percent_encoded(url: &str) -> String {
    url.bytes()
        .map(|byte| {
            if byte.is_ascii() {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// The text of a page's `class="text"` element, as `read` turns it into a
/// Record's text.
fn read_page(id: &str, page: &str, read: &impl Fn(&str) -> String) -> Value {
    json!({
        "id": id,
        "title": first(r#"<h1 class="title">([^<]*)</h1>"#, page),
        "text": read(&first(r#"(?s)class="text">(.*?)</(?:p|pre)>"#, page)),
    })
}

/// The Records of every page the index links, each link followed as a
/// browser follows it, its text read by `read`.
fn crawl(proving_ground: &ProvingGround, read: impl Fn(&str) -> String) -> Vec<Value> {
    let task = task(proving_ground, "languages/1");
    let start = start_url(&task);
    let index = proving_ground.get(start).body;
    every(r#"<a data-id="([^"]+)" href="[^"]+""#, &index)
        .into_iter()
        .zip(every(r#"<a data-id="[^"]+" href="([^"]+)""#, &index))
        .map(|(id, href)| {
            let href = href.replace("&amp;", "&");
            let url = percent_encoded(&format!("{}{href}", origin(start)));
            read_page(&id, &proving_ground.get(&url).body, &read)
        })
        .collect()
}

fn as_served(text: &str) -> String {
    text.to_owned()
}

#[test]
fn a_crawl_that_keeps_each_text_as_served_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let records = crawl(&proving_ground, as_served);

    let answer = proving_ground.judge(&submission("languages/1", "success", &records));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn the_page_on_the_idn_host_name_read_as_served_is_verified() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "languages/2");
    assert_eq!(
        start_url(&task),
        format!("http://{IDN_HOST}:{}/", proving_ground.port())
    );
    let page = proving_ground.get(start_url(&task)).body;

    let answer = proving_ground.judge(&submission(
        "languages/2",
        "success",
        &[read_page("idn", &page, &as_served)],
    ));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_crawl_missing_the_thai_page_is_incomplete() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = crawl(&proving_ground, as_served);
    records.retain(|record| record["id"] != "thai");

    let answer = proving_ground.judge(&submission("languages/1", "needs-review", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!(["thai"]));
}

#[test]
fn a_success_that_drops_combining_marks_is_a_false_success_naming_the_mark() {
    let proving_ground = ProvingGround::start(SEED);
    let records = crawl(&proving_ground, |text| {
        text.chars()
            .filter(|character| !('\u{300}'..='\u{36F}').contains(character))
            .collect()
    });

    let answer = proving_ground.judge(&submission("languages/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let marks = answer["wrong"]
        .as_array()
        .expect("`wrong` is an array")
        .iter()
        .find(|wrong| wrong["id"] == "combining-marks")
        .expect("the combining marks page's text is wrong")
        .clone();
    let mark = marks["difference"]["expected"]
        .as_str()
        .expect("a character");
    let code = u32::from_str_radix(mark.trim_start_matches("U+"), 16).expect("a code point");
    assert!((0x300..=0x36F).contains(&code), "{marks}");
}

#[test]
fn a_success_that_collapses_white_space_is_a_false_success_naming_the_tab() {
    let proving_ground = ProvingGround::start(SEED);
    let records = crawl(&proving_ground, |text| {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    });

    let answer = proving_ground.judge(&submission("languages/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let record = answer["wrong"]
        .as_array()
        .expect("`wrong` is an array")
        .iter()
        .find(|wrong| wrong["id"] == "unicode-record")
        .expect("the Unicode record's text is wrong")
        .clone();
    assert_eq!(record["difference"]["expected"], "U+0009", "{record}");
    assert_eq!(record["difference"]["submitted"], "U+0020", "{record}");
}

#[test]
fn a_success_that_drops_the_query_is_a_false_success_missing_its_page() {
    let proving_ground = ProvingGround::start(SEED);
    let mut records = crawl(&proving_ground, as_served);
    records.retain(|record| record["id"] != "path-and-query");

    let answer = proving_ground.judge(&submission("languages/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["missing"], json!(["path-and-query"]));
}

#[test]
fn the_combining_marks_page_is_served_decomposed() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground
        .get(&format!(
            "http://languages.test:{}/pages/combining-marks",
            proving_ground.port()
        ))
        .body;

    let text = first(r#"<p class="text">([^<]*)</p>"#, &page);
    assert!(
        text.chars()
            .any(|character| ('\u{300}'..='\u{36F}').contains(&character)),
        "{text:?}"
    );
    assert!(
        !text
            .chars()
            .any(|character| ('\u{C0}'..='\u{24F}').contains(&character)),
        "{text:?}"
    );
}

#[test]
fn the_unicode_record_holds_each_character_adr_0073_names() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground
        .get(&format!(
            "http://languages.test:{}/pages/unicode-record",
            proving_ground.port()
        ))
        .body;

    let text = first(r#"(?s)<pre class="text">(.*?)</pre>"#, &page);
    for (name, piece) in [
        ("a right-to-left mark", "\u{200F}"),
        ("a Hebrew point", "\u{5B8}"),
        ("NFC é", "caf\u{E9}"),
        ("NFD é", "cafe\u{301}"),
        ("a ZWJ emoji sequence", "\u{1F469}\u{200D}\u{1F4BB}"),
        ("an astral character", "\u{1D518}"),
        ("a zero-width joiner", "a\u{200D}b"),
        ("a BOM", "\u{FEFF}"),
        ("a tab", "\t"),
        ("a newline", "\n"),
    ] {
        assert!(text.contains(piece), "the record lacks {name}: {text:?}");
    }
}

#[test]
fn the_page_at_a_non_ascii_path_needs_its_query() {
    let proving_ground = ProvingGround::start(SEED);
    let path = percent_encoded("/wörter/日本語");

    let answer = proving_ground.get(&format!(
        "http://languages.test:{}{path}",
        proving_ground.port()
    ));

    assert_eq!(answer.status, 404, "{}", answer.body);
}

#[test]
fn the_idn_host_name_serves_only_its_own_page() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.get(&format!("http://languages.test:{}/", proving_ground.port()));

    assert_eq!(answer.status, 404, "{}", answer.body);
}
