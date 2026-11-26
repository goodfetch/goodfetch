//! The Extractor's cases (`conformance/extract/cases.toml`): the article
//! page each case sends with `extract.content`, and the checks of what an
//! implementation answers for it.

use goodfetch_protocol::plugin::extractor::{
    EXTRACT_CONTENT, ExtractContentResult, is_metadata_key,
};
use goodfetch_protocol::result_document::CaseCheck;
use serde::Deserialize as _;
use serde_json::{Value, json};

/// The method every case calls.
pub(super) const METHOD: &str = EXTRACT_CONTENT;

/// The article page's URL.
const URL: &str = "https://shop.test/kettle";

/// A sentence of the article, which its main content holds.
const SENTENCE: &str = "The kettle boils a litre of water in three minutes";

/// A sentence of the page's navigation, which its main content does not
/// hold.
const NAVIGATION: &str = "Browse every kitchen appliance in the shop";

/// The article page: a navigation and a footer around one article.
fn article() -> String {
    format!(
        "<!doctype html><html><head><title>Kettle</title></head><body>\
         <nav><a href=\"/\">Home</a> <a href=\"/kitchen\">{NAVIGATION}</a></nav>\
         <main><article><h1>Kettle</h1>\
         <p>{SENTENCE}, and switches itself off once the water boils.</p>\
         <p>Its steel body stays cool to the touch, and its filter keeps limescale out of every cup you pour from it.</p>\
         <p>The base turns all the way round, so the kettle sits on it whichever hand lifts it, and the cord tucks away underneath.</p>\
         <p>A window on the side shows how much water is left, marked from half a litre up to a litre and a half.</p>\
         </article></main>\
         <footer><a href=\"/contact\">Contact</a></footer></body></html>"
    )
}

/// The params of `extract.content` for the article page, with a field a
/// newer engine adds when `newer`.
pub(super) fn params(newer: bool) -> Value {
    let mut params = json!({ "url": URL, "html": article() });
    if newer {
        params["added_in_a_minor"] = json!(1);
    }
    params
}

/// The checks of `answer`, what an implementation answered for the
/// article page.
pub(super) fn checks(answer: &Value) -> Vec<CaseCheck> {
    let result = answer
        .get("result")
        .and_then(|result| ExtractContentResult::deserialize(result).ok());
    let holds = |text: &str| {
        result
            .as_ref()
            .is_some_and(|result| result.text.contains(text) || result.markdown.contains(text))
    };
    vec![
        CaseCheck {
            name: "the answer is `{ title, text, markdown, metadata }` and no other field, its metadata keys `snake_case`".to_owned(),
            passed: result.as_ref().is_some_and(|result| {
                result.metadata.keys().all(|key| is_metadata_key(key))
            }),
        },
        CaseCheck {
            name: "its text and its Markdown hold the article's sentence".to_owned(),
            passed: result.as_ref().is_some_and(|result| {
                result.text.contains(SENTENCE) && result.markdown.contains(SENTENCE)
            }),
        },
        CaseCheck {
            name: "neither holds the page's navigation".to_owned(),
            passed: result.is_some() && !holds(NAVIGATION),
        },
    ]
}
