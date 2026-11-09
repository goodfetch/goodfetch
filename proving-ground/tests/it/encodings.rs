//! The `encodings` family (ADRs 0073 and 0119): pages written in legacy
//! encodings and in UTF-16, declared by a BOM, a header, a `<meta>` or not
//! at all, whose expected text the oracle holds in UTF-8. Each page is read
//! here by the encoding ADR 0119's order gives it, written below by hand, so
//! a known-right result is the text a browser shows.
//! Covers: story:2-33, story:2-35, story:2-90

use encoding_rs::Encoding;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::{ProvingGround, submission};
use crate::reading::{every, first, origin, start_url, task};

const SEED: u64 = 165;

/// Each page: its ID, the `Content-Type` it is served with, the encoding
/// ADR 0119's order reads it with, and where that encoding comes from.
const PAGES: [(&str, &str, &str, &str); 18] = [
    (
        "shift-jis",
        "text/html; charset=Shift_JIS",
        "Shift_JIS",
        "header",
    ),
    ("euc-kr", "text/html; charset=EUC-KR", "EUC-KR", "header"),
    ("gb18030", "text/html", "gb18030", "meta"),
    ("big5", "text/html; charset=Big5", "Big5", "header"),
    ("windows-1251", "text/html", "windows-1251", "meta"),
    (
        "iso-8859-2",
        "text/html; charset=ISO-8859-2",
        "ISO-8859-2",
        "header",
    ),
    (
        "iso-8859-15",
        "text/html; charset=ISO-8859-15",
        "ISO-8859-15",
        "header",
    ),
    ("utf-16-bom", "text/html", "UTF-16LE", "bom"),
    (
        "contradicting-meta",
        "text/html; charset=windows-1251",
        "windows-1251",
        "header",
    ),
    ("undeclared-utf-8", "text/html", "UTF-8", "utf8"),
    ("undeclared-legacy", "text/html", "Shift_JIS", "detected"),
    (
        "latin1-label",
        "text/html; charset=latin1",
        "windows-1252",
        "header",
    ),
    ("ascii-label", "text/html", "windows-1252", "meta"),
    ("meta-utf-16-label", "text/html", "UTF-8", "meta"),
    ("x-user-defined", "text/html", "windows-1252", "meta"),
    ("late-meta-in-head", "text/html", "windows-1251", "meta"),
    ("meta-after-body", "text/html", "UTF-8", "utf8"),
    (
        "undecodable-byte",
        "text/html; charset=utf-8",
        "UTF-8",
        "header",
    ),
];

/// A page's bytes and `Content-Type`.
fn raw_page(proving_ground: &ProvingGround, id: &str) -> (Option<String>, Vec<u8>) {
    let answer = proving_ground.get_raw(&format!(
        "http://encodings.test:{}/pages/{id}",
        proving_ground.port()
    ));
    assert_eq!(answer.status, 200, "{id}");
    (answer.content_type, answer.body)
}

/// The Record a page shows, decoded as `encoding`, with the encoding and
/// its source when `provenance` asks for them.
fn read_page(url: &str, body: &[u8], encoding: &str, source: &str, provenance: bool) -> Value {
    let encoding = Encoding::for_label(encoding.as_bytes()).expect("a WHATWG label");
    let (text, _, _) = encoding.decode(body);
    let id = url.rsplit('/').next().expect("a page path");
    let mut record = json!({
        "id": id,
        "title": first(r#"<h1 class="title">([^<]*)</h1>"#, &text),
        "text": first(r#"(?s)<p class="text">(.*?)</p>"#, &text),
    });
    if provenance {
        record["encoding"] = json!(encoding.name());
        record["encoding_source"] = json!(source);
    }
    record
}

/// The Records of every page the index links, each read by `decode`.
fn crawl(
    proving_ground: &ProvingGround,
    task: &Value,
    decode: impl Fn(&str, &[u8]) -> Value,
) -> Vec<Value> {
    let start = start_url(task);
    let index = proving_ground.get(start).body;
    every(r#"<a href="(/pages/[^"]+)""#, &index)
        .iter()
        .map(|path| {
            let url = format!("{}{path}", origin(start));
            decode(&url, &proving_ground.get_raw(&url).body)
        })
        .collect()
}

/// The encoding and source the table gives a page's URL.
fn declared(url: &str) -> (&'static str, &'static str) {
    let id = url.rsplit('/').next().expect("a page path");
    let (_, _, encoding, source) = PAGES
        .iter()
        .find(|page| page.0 == id)
        .unwrap_or_else(|| panic!("the index links a page the table lacks: {id}"));
    (encoding, source)
}

/// A known-right result: each page read as ADR 0119 reads it.
fn known_right(proving_ground: &ProvingGround, task: &Value, provenance: bool) -> Vec<Value> {
    crawl(proving_ground, task, |url, body| {
        let (encoding, source) = declared(url);
        read_page(url, body, encoding, source, provenance)
    })
}

#[rstest]
#[case::text("encodings/1", false)]
#[case::text_and_its_provenance("encodings/2", true)]
fn each_page_read_by_its_declared_encoding_is_verified(#[case] id: &str, #[case] provenance: bool) {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, id);
    let records = known_right(&proving_ground, &task, provenance);

    let answer = proving_ground.judge(&submission(id, "success", &records));

    assert_eq!(records.len(), PAGES.len());
    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn a_crawl_missing_a_page_is_incomplete_naming_it() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "encodings/1");
    let mut records = known_right(&proving_ground, &task, false);
    records.retain(|record| record["id"] != "big5");

    let answer = proving_ground.judge(&submission("encodings/1", "failure", &records));

    assert_eq!(answer["outcome"], "incomplete", "{answer}");
    assert_eq!(answer["missing"], json!(["big5"]));
}

#[test]
fn a_success_that_reads_every_page_as_utf_8_is_a_false_success_naming_the_characters() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "encodings/1");
    let records = crawl(&proving_ground, &task, |url, body| {
        read_page(url, body, "UTF-8", "utf8", false)
    });

    let answer = proving_ground.judge(&submission("encodings/1", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let wrong = answer["wrong"].as_array().expect("`wrong` is an array");
    let shift_jis_title = wrong
        .iter()
        .find(|wrong| wrong["id"] == "shift-jis" && wrong["field"] == "title")
        .expect("the Shift_JIS page's title is wrong");
    assert_eq!(shift_jis_title["expected"], "日本語");
    assert_eq!(
        shift_jis_title["difference"],
        json!({ "at": 0, "expected": "U+65E5", "submitted": "U+FFFD" })
    );
    assert!(
        wrong.iter().all(|wrong| wrong["id"] != "undeclared-utf-8"),
        "{answer}"
    );
}

#[test]
fn a_success_that_names_the_wrong_source_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);
    let task = task(&proving_ground, "encodings/2");
    let mut records = known_right(&proving_ground, &task, true);
    let late = records
        .iter_mut()
        .find(|record| record["id"] == "contradicting-meta")
        .expect("the page with a contradicting meta");
    late["encoding_source"] = json!("meta");

    let answer = proving_ground.judge(&submission("encodings/2", "success", &records));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["wrong"][0]["field"], "encoding_source", "{answer}");
    assert_eq!(answer["wrong"][0]["expected"], "header", "{answer}");
}

#[test]
fn each_page_is_served_with_the_content_type_its_case_declares() {
    let proving_ground = ProvingGround::start(SEED);

    let served: Vec<(&str, Option<String>)> = PAGES
        .iter()
        .map(|(id, _, _, _)| (*id, raw_page(&proving_ground, id).0))
        .collect();

    let declared: Vec<(&str, Option<String>)> = PAGES
        .iter()
        .map(|(id, content_type, _, _)| (*id, Some((*content_type).to_owned())))
        .collect();
    assert_eq!(served, declared);
}

#[test]
fn the_shift_jis_page_holds_its_title_in_shift_jis_bytes() {
    let proving_ground = ProvingGround::start(SEED);

    let (_, body) = raw_page(&proving_ground, "shift-jis");

    // 日本語 in Shift_JIS.
    let title = [0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA];
    assert!(body.windows(title.len()).any(|window| window == title));
}

#[test]
fn the_utf_16_page_starts_with_its_little_endian_bom() {
    let proving_ground = ProvingGround::start(SEED);

    let (_, body) = raw_page(&proving_ground, "utf-16-bom");

    assert_eq!(body[..4], [0xFF, 0xFE, b'<', 0x00]);
}

#[rstest]
#[case::latin1("latin1-label")]
#[case::ascii("ascii-label")]
#[case::x_user_defined("x-user-defined")]
fn a_page_labelled_as_windows_1252_holds_the_euro_as_byte_0x80(#[case] id: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let (_, body) = raw_page(&proving_ground, id);

    assert!(body.contains(&0x80), "{id}");
}

/// Where the page's `<meta charset>` starts, and where its `<body>` does.
fn meta_and_body(body: &[u8]) -> (usize, usize) {
    let find = |needle: &[u8]| {
        body.windows(needle.len())
            .position(|window| window == needle)
            .expect("the page holds the tag")
    };
    (find(b"<meta charset="), find(b"<body"))
}

#[test]
fn the_late_meta_comes_after_the_first_1024_bytes_among_head_tags() {
    let proving_ground = ProvingGround::start(SEED);

    let (_, body) = raw_page(&proving_ground, "late-meta-in-head");

    let (meta, body_tag) = meta_and_body(&body);
    assert!(meta > 1024 && meta < body_tag, "{meta}, {body_tag}");
}

#[test]
fn the_meta_after_the_body_tag_comes_after_the_first_1024_bytes() {
    let proving_ground = ProvingGround::start(SEED);

    let (_, body) = raw_page(&proving_ground, "meta-after-body");

    let (meta, body_tag) = meta_and_body(&body);
    assert!(meta > 1024 && meta > body_tag, "{meta}, {body_tag}");
}

#[test]
fn the_undecodable_page_holds_a_byte_utf_8_never_writes() {
    let proving_ground = ProvingGround::start(SEED);

    let (_, body) = raw_page(&proving_ground, "undecodable-byte");

    assert!(body.contains(&0xFF));
}

#[test]
fn another_seed_writes_other_text() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED + 1);

    let pages = (
        raw_page(&first, "shift-jis").1,
        raw_page(&second, "shift-jis").1,
    );

    assert_ne!(pages.0, pages.1);
}

/// The XHTML page no index links: `application/xhtml+xml` in windows-1251,
/// declared only in its XML declaration (ADR 0119).
#[test]
fn the_xhtml_page_declares_its_encoding_only_in_its_xml_declaration() {
    let proving_ground = ProvingGround::start(SEED);
    let answer = proving_ground.get_raw(&format!(
        "http://encodings.test:{}/xhtml/declared",
        proving_ground.port()
    ));
    let (text, _, malformed) = encoding_rs::WINDOWS_1251.decode(&answer.body);

    assert_eq!(
        (answer.status, answer.content_type.as_deref(), malformed),
        (200, Some("application/xhtml+xml"), false)
    );
    assert!(
        text.starts_with("<?xml version=\"1.0\" encoding=\"windows-1251\"?>"),
        "{text}"
    );
    assert!(!text.contains("<meta"), "{text}");
    assert_eq!(
        first(r#"(?s)<p class="text">(.*?)</p>"#, &text),
        "Москва — столица России."
    );
    assert!(std::str::from_utf8(&answer.body).is_err());
}
