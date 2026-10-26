//! HTML judged by its blocks of text.

use rstest::rstest;

use super::{same_html, same_text};

#[rstest]
#[case::a_wrapper_and_absolute_links(
    "<p>Read <a href=\"/a\">this</a>.</p><ul><li>One</li><li>Two</li></ul>",
    "<div id=\"page\"><div><p>Read <a href=\"https://a.test/a\">this</a>.</p>\n<ul>\n<li>One</li>\n<li>Two</li></ul></div></div>",
    true
)]
#[case::a_whole_document_and_its_body(
    "<!doctype html><html><head><title>T</title><script>x()</script></head><body><h1>A</h1><p>B</p></body></html>",
    "<h1>A</h1><p>B</p>",
    true
)]
#[case::a_paragraph_lost("<p>A</p><p>B</p>", "<p>A</p>", false)]
#[case::two_paragraphs_made_one("<p>A</p><p>B</p>", "<p>A B</p>", false)]
#[case::other_text("<p>A kite</p>", "<p>A Kite</p>", false)]
fn html_is_the_same_when_its_blocks_of_text_are(
    #[case] expected: &str,
    #[case] submitted: &str,
    #[case] same: bool,
) {
    assert_eq!(same_html(expected, submitted), same);
}

#[rstest]
#[case::white_space_aside("A kite\n\nflies.\n", "A kite flies.", true)]
#[case::a_word_lost("A kite flies.", "A kite.", false)]
fn text_is_the_same_white_space_aside(
    #[case] expected: &str,
    #[case] submitted: &str,
    #[case] same: bool,
) {
    assert_eq!(same_text(expected, submitted), same);
}
