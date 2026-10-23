//! Markdown read as its structure, the oracle's way to judge a Markdown
//! value (ADR 0007, after the lab's Markdown oracle): a document is reduced
//! to its headings, paragraphs, list items, table cells and code blocks,
//! each as its text, and its links, each as its text and its target. Two
//! documents of the same structure are the same Markdown whatever syntax
//! each converter chose: ATX or setext headings, `-` or `*` bullets,
//! fenced or indented code, escapes or none, a heading's level, a code
//! block's language, a link written relative or absolute. Text is kept as
//! written, its case and normalization form included (ADR 0073); only its
//! white space is collapsed, as Markdown's own reading does.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use url::Url;

/// What a Markdown document holds, each kind of element in document order.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Structure {
    headings: Vec<String>,
    paragraphs: Vec<String>,
    list_items: Vec<String>,
    table_cells: Vec<String>,
    code_blocks: Vec<String>,
    /// `text -> target`, the target resolved against the page and written
    /// as its path when it is on the page's site.
    links: Vec<String>,
}

/// An element whose text is being read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    Heading,
    Paragraph,
    ListItem,
    TableCell,
    CodeBlock,
    /// An image, whose alternative text is no text of the page's.
    Image,
}

/// Whether `submitted` is the same Markdown as `expected`, for the page at
/// `page`, whose site is every host that is `site` or ends in `.{site}`.
pub(crate) fn same_markdown(expected: &str, submitted: &str, page: &Url, site: &str) -> bool {
    structure(expected, page, site) == structure(submitted, page, site)
}

/// The structure of `markdown`, a page's at `page`.
pub(crate) fn structure(markdown: &str, page: &Url, site: &str) -> Structure {
    let mut structure = Structure::default();
    let mut reading: Vec<(Reading, String)> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES) {
        match event {
            Event::Start(tag) => {
                let started = match tag {
                    Tag::Heading { .. } => Some(Reading::Heading),
                    // A list item's paragraphs are the item's own text.
                    Tag::Paragraph
                        if reading.last().map(|(kind, _)| *kind) == Some(Reading::ListItem) =>
                    {
                        None
                    }
                    Tag::Paragraph => Some(Reading::Paragraph),
                    Tag::Item => Some(Reading::ListItem),
                    Tag::TableCell => Some(Reading::TableCell),
                    Tag::CodeBlock(_) => Some(Reading::CodeBlock),
                    Tag::Image { .. } => Some(Reading::Image),
                    Tag::Link { dest_url, .. } => {
                        links.push((resolved(&dest_url, page, site), String::new()));
                        None
                    }
                    _ => None,
                };
                if let Some(kind) = started {
                    reading.push((kind, String::new()));
                }
            }
            Event::End(tag) => {
                let ended = match tag {
                    TagEnd::Heading(_) => Some(Reading::Heading),
                    TagEnd::Paragraph => Some(Reading::Paragraph),
                    TagEnd::Item => Some(Reading::ListItem),
                    TagEnd::TableCell => Some(Reading::TableCell),
                    TagEnd::CodeBlock => Some(Reading::CodeBlock),
                    TagEnd::Image => Some(Reading::Image),
                    TagEnd::Link => {
                        if let Some((target, text)) = links.pop() {
                            structure
                                .links
                                .push(format!("{} -> {target}", collapsed(&text)));
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(kind) = ended
                    && reading.last().map(|(open, _)| *open) == Some(kind)
                    && let Some((kind, text)) = reading.pop()
                {
                    structure.keep(kind, &text);
                }
            }
            Event::Text(text) | Event::Code(text) => {
                read(&mut reading, &mut links, &text);
            }
            Event::SoftBreak | Event::HardBreak => read(&mut reading, &mut links, "\n"),
            _ => {}
        }
    }
    structure
}

/// Adds `text` to the element being read, and to each link it is in.
fn read(reading: &mut [(Reading, String)], links: &mut [(String, String)], text: &str) {
    if let Some((kind, read)) = reading.last_mut()
        && *kind != Reading::Image
    {
        read.push_str(text);
        for (_, link) in links.iter_mut() {
            link.push_str(text);
        }
    }
}

impl Structure {
    fn keep(&mut self, kind: Reading, text: &str) {
        let (list, text) = match kind {
            Reading::Heading => (&mut self.headings, collapsed(text)),
            Reading::Paragraph => (&mut self.paragraphs, collapsed(text)),
            Reading::ListItem => (&mut self.list_items, collapsed(text)),
            Reading::TableCell => (&mut self.table_cells, collapsed(text)),
            Reading::CodeBlock => (
                &mut self.code_blocks,
                text.lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim_matches('\n')
                    .to_owned(),
            ),
            Reading::Image => return,
        };
        if !text.is_empty() {
            list.push(text);
        }
    }
}

/// `text` with each run of white space one space, and none at its ends.
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A link's target resolved against `page`: its path, query and fragment
/// when it is on the page's site, else the whole URL; as written when it
/// does not resolve.
fn resolved(target: &str, page: &Url, site: &str) -> String {
    let Ok(url) = page.join(target) else {
        return target.to_owned();
    };
    let on_site = url
        .host_str()
        .is_some_and(|host| host == site || host.ends_with(&format!(".{site}")));
    if on_site {
        url[url::Position::BeforePath..].to_owned()
    } else {
        url.to_string()
    }
}
