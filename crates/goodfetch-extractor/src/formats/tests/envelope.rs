//! What a call answers: the formats it named and no other, with the
//! document's URL, its base URL and whether it has main content.

use goodfetch_protocol::page::MainContent;
use serde_json::json;

use super::{URL, read};

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
            MainContent::Found,
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
        (MainContent::None, Some(""))
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
