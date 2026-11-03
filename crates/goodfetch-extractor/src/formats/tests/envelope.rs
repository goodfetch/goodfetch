//! What a call answers: the formats it named and no other, with the
//! document's URL, its base URL and whether it has main content.

use std::sync::mpsc::{self, Receiver, Sender};

use goodfetch_protocol::page::{FormatsRequest, MainContent};
use serde_json::json;

use super::{URL, read};
use crate::{BuiltIn, Context, Document, Extractor, Serialization, formats};

/// Covers: story:6-1
#[test]
fn an_articles_main_content_is_read_as_markdown() {
    let read = read(
        include_str!("../fixtures/article.html"),
        json!({ "markdown": true }),
    );

    assert_eq!(
        (read.main_content, read.markdown.as_deref()),
        (
            Some(MainContent::Found),
            Some(
                "## How a kite stays in the air\n\nA kite flies because the wind pushes on its sail at an angle, and the line holds it against that push. The air that flows under the sail is slowed and pressed, while the air over the top speeds up, so the kite is lifted as well as pushed.\n\n## The bridle\n\nThe bridle is the set of lines that joins the flying line to the frame. Moving its towing point forward makes the kite fly flatter and faster; moving it back makes it climb steeply and pull harder on the hand.\n\n- Light wind: towing point back.\n- Strong wind: towing point forward.\n\nMost flyers adjust the bridle a little at a time, letting the kite settle for a minute after each change before judging the result.\n"
            )
        )
    );
}

/// Covers: story:6-8
#[test]
fn a_format_the_request_does_not_name_is_left_out() {
    let read = read(include_str!("../fixtures/article.html"), json!({}));

    assert_eq!(read.markdown, None);
}

/// Covers: story:6-12
#[test]
fn a_document_with_no_main_content_answers_empty_markdown_and_none() {
    let read = read(
        include_str!("../fixtures/no-main-content.html"),
        json!({ "markdown": true }),
    );

    assert_eq!(
        (read.main_content, read.markdown.as_deref()),
        (Some(MainContent::None), Some(""))
    );
}

/// A main-content step that reports each of its runs.
struct Counted {
    runs: Sender<()>,
}

impl Counted {
    /// The step, and where its runs are reported.
    fn new() -> (Self, Receiver<()>) {
        let (runs, reported) = mpsc::channel();
        (Self { runs }, reported)
    }
}

impl Extractor for Counted {
    fn main_content(&self, html: &str, url: &str) -> Option<String> {
        self.runs.send(()).unwrap();
        BuiltIn::default().extractor().main_content(html, url)
    }
}

/// A call whose formats none reads from the main content costs no
/// main-content step, and answers no `main_content` (story 8).
/// Covers: story:6-8
#[test]
fn a_request_with_no_format_of_the_main_content_never_runs_the_step_and_answers_no_main_content() {
    let (counted, runs) = Counted::new();
    let request: FormatsRequest = serde_json::from_value(json!({
        "markdown": { "content": "full" },
        "html": { "kind": "cleaned" },
        "links": true,
        "images": true,
        "media": true,
        "tables": true,
        "metadata": true,
        "contacts": true,
    }))
    .unwrap();

    let read = formats(
        &Document {
            html: include_str!("../fixtures/article.html"),
            url: URL,
            base_url: URL,
            serialization: Serialization::Served,
        },
        &request,
        &Context {
            clean_up: None,
            extractor: &counted,
            work: goodfetch_counted_work::CountedWork::default(),
        },
    );

    assert_eq!(
        (
            serde_json::to_value(read)
                .unwrap()
                .get("main_content")
                .cloned(),
            runs.try_iter().count()
        ),
        (None, 0)
    );
}

#[test]
fn the_documents_url_and_base_url_are_answered_as_given() {
    let read = read(
        include_str!("../fixtures/article.html"),
        json!({ "markdown": true }),
    );

    assert_eq!((read.url.as_str(), read.base_url.as_str()), (URL, URL));
}
