//! The fetch tiers' benchmark scenarios (ADR 0071), shared by the Callgrind
//! and DHAT targets: the decoding order of ADR 0119 over a pinned corpus of
//! documents, each found its encoding and decoded as a non-browser tier
//! reads a crawl request's document, with no network, no sleep and no
//! timing. The order's pure modules are compiled in through `#[path]`
//! (TESTING.md, Benchmarks).

#[path = "../src/charset.rs"]
#[expect(
    dead_code,
    unreachable_pub,
    reason = "the scenario runs the order and its decoding, never the fuzz target's body, which the crate exports"
)]
mod charset;
#[path = "../src/sniffing.rs"]
#[expect(
    unreachable_pub,
    reason = "the crate exports the sniffer, which the scenario calls in place"
)]
mod sniffing;

use charset::{decode, read};
use goodfetch_encoding::{Kind, content_type};

/// `line` repeated past the first 1,024 bytes of a document.
fn past_1024(line: &str) -> String {
    line.repeat(1024 / line.len() + 2)
}

/// A document of about 8 KiB: `head` in its head, then paragraphs of
/// `text`, written in `encoding`.
fn document(head: &str, text: &str, encoding: &'static encoding_rs::Encoding) -> Vec<u8> {
    let body = format!("<p>{text}</p>\n").repeat(8192 / (text.len() + 8));
    let html = format!(
        "<!doctype html>\n<html><head>{head}<title>Bench</title></head>\n<body><main>{body}</main></body></html>\n"
    );
    encoding.encode(&html).0.into_owned()
}

/// The corpus: each document with its `Content-Type`, one per step of the
/// order and the guess that escalates.
pub(crate) fn corpus() -> Vec<(Vec<u8>, Option<&'static str>)> {
    let japanese = "東京は日本の首都です。今日はいい天気ですね。";
    let russian = "Москва — столица России.";
    let mut bom = b"\xEF\xBB\xBF".to_vec();
    bom.extend(document("", japanese, encoding_rs::UTF_8));
    vec![
        (bom, Some("text/html")),
        (
            document("", japanese, encoding_rs::SHIFT_JIS),
            Some("text/html; charset=Shift_JIS"),
        ),
        (
            document(
                "<meta charset=\"windows-1251\">",
                russian,
                encoding_rs::WINDOWS_1251,
            ),
            Some("text/html"),
        ),
        (
            document(
                &format!(
                    "{}<meta charset=\"windows-1251\">",
                    past_1024("<meta name=\"keywords\" content=\"bench\">")
                ),
                russian,
                encoding_rs::WINDOWS_1251,
            ),
            Some("text/html"),
        ),
        (
            document("", japanese, encoding_rs::UTF_8),
            Some("text/html"),
        ),
        (document("", japanese, encoding_rs::UTF_8), None),
        (
            document("", japanese, encoding_rs::SHIFT_JIS),
            Some("text/html"),
        ),
    ]
}

/// Finds each document's encoding and decodes it as a non-browser tier
/// does, a response with no `Content-Type` typed by sniffing; answers the
/// characters decoded and how many documents only a guess reads.
pub(crate) fn decode_all(corpus: &[(Vec<u8>, Option<&'static str>)]) -> (usize, usize) {
    let mut characters = 0;
    let mut guesses = 0;
    for (body, header) in corpus {
        let (essence, charset) = header.map_or_else(
            || (sniffing::sniff(body, "/").to_owned(), None),
            content_type,
        );
        match read(body, Kind::of(&essence), charset, None) {
            Some(found) => characters += decode(body, found).chars().count(),
            None => guesses += 1,
        }
    }
    (characters, guesses)
}
