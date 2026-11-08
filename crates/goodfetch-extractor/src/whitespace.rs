//! White space collapsed as the HTML Standard's "strip and collapse ASCII
//! whitespace" does for a document's title: the one rule the readers of a
//! title, a link's text and a table cell's text share, so a page's
//! `metadata.title` and its signature's title never differ.

/// `text` with each run of ASCII white space made one space, none at
/// either end; any other space, such as a no-break space, kept.
pub(crate) fn collapsed(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
