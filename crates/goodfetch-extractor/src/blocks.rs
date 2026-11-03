//! A document read as blocks of text: its body without its head, the main
//! content's text a paragraph per block, and the main content "fit"
//! (`Crawl4AI`'s meaning, spec #580): without its link-dense blocks, those
//! with more than half their text in links, its image-only blocks and the
//! navigation lists the document repeats, each link compared by its target
//! resolved against the document's base URL. Lengths are counted in
//! characters, white space aside.

use dom_query::NodeRef;
use url::{Position, Url};

/// The elements that hold no text a person reads: `noscript`, and those
/// the HTML Standard's rendering hides that can hold text.
const NOT_TEXT: [&str; 10] = [
    "datalist", "head", "noembed", "noframes", "noscript", "rp", "script", "style", "template",
    "title",
];

/// The elements that end the paragraph before and after them: those the
/// HTML Standard's rendering lays out as blocks, list items, table parts
/// and a list box's rows. Every other element, a phrasing one or one the
/// Standard does not name, runs inside its paragraph.
const BLOCK: [&str; 55] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "html",
    "legend",
    "li",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "optgroup",
    "option",
    "p",
    "plaintext",
    "pre",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "xmp",
];

/// The blocks "fit" may leave out; headings and code are kept whatever
/// their links.
const CANDIDATE: &str = "p, li, ul, ol, menu, dl, dd, dt, table, blockquote, figure, div, \
     section, aside, nav, header, footer, form, details";

/// What shows an image.
const IMAGE: &str = "img, picture, svg, video";

/// The lists a "repeated navigation list" is one of.
const LIST: &str = "ul, ol, menu";

/// The HTML of `html`'s body, without its head: the whole page a person
/// reads.
#[expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a raw parse no benchmark reaches yet; the Shared paths audit moves it onto `CountedWork::parse_html`"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(crate) fn body(html: &str) -> String {
    let document = dom_query::Document::from(html);
    document.select("body").inner_html().to_string()
}

/// The text of `html`, each block's text a paragraph, its white space made
/// one space, the paragraphs one blank line apart.
#[expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a raw parse no benchmark reaches yet; the Shared paths audit moves it onto `CountedWork::parse_html`"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(crate) fn text(html: &str) -> String {
    let document = dom_query::Document::from(html);
    let mut blocks = Blocks::default();
    for node in document.select("html").nodes() {
        blocks.read(node);
    }
    blocks.end();
    blocks.done.join("\n\n")
}

#[derive(Default)]
struct Blocks {
    done: Vec<String>,
    current: String,
}

impl Blocks {
    fn read(&mut self, node: &NodeRef<'_>) {
        if node.is_text() {
            self.current.push_str(&node.text());
            return;
        }
        if !node.is_element() {
            return;
        }
        let name = node.node_name().unwrap_or_default();
        let name = name.as_ref();
        if NOT_TEXT.contains(&name) {
            return;
        }
        if name == "br" {
            self.end();
            return;
        }
        let block = BLOCK.contains(&name);
        if block {
            self.end();
        }
        for child in node.children_it(false) {
            self.read(&child);
        }
        if block {
            self.end();
        }
    }

    fn end(&mut self) {
        let block = self
            .current
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !block.is_empty() {
            self.done.push(block);
        }
        self.current.clear();
    }
}

/// `main`, the main content's HTML, without its link-dense blocks, its
/// image-only blocks and the lists whose links another list of `document`
/// (the whole page, whose base URL is `base_url`) repeats, in order.
#[expect(
    clippy::disallowed_types,
    reason = "ADR 0140: a raw parse no benchmark reaches yet; the Shared paths audit moves it onto `CountedWork::parse_html`"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(crate) fn fit(main: &str, document: &str, base_url: &str) -> String {
    let base = Url::parse(base_url).ok();
    let page = dom_query::Document::from(document);
    let page_lists: Vec<Vec<String>> = page
        .select(LIST)
        .nodes()
        .iter()
        .map(|list| targets(list, base.as_ref()))
        .filter(|targets| !targets.is_empty())
        .collect();
    let lists = PageLists {
        base: base.as_ref(),
        targets: &page_lists,
    };
    let content = dom_query::Document::from(main);
    let root = content.select("body");
    for node in root.nodes() {
        prune(node, &lists);
    }
    root.inner_html().to_string()
}

/// The page's lists, each as its links' targets, and the base URL they
/// were resolved against.
struct PageLists<'a> {
    base: Option<&'a Url>,
    targets: &'a [Vec<String>],
}

/// Takes out of `node`'s descendants, from the outermost, each candidate
/// block "fit" leaves out.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn prune(node: &NodeRef<'_>, lists: &PageLists<'_>) {
    for child in node.element_children() {
        if child.is(CANDIDATE) && left_out(&child, lists) {
            child.remove_from_parent();
        } else {
            prune(&child, lists);
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn left_out(block: &NodeRef<'_>, lists: &PageLists<'_>) -> bool {
    let all = length(&block.text());
    if all == 0 {
        return block.descendants_it().any(|node| node.is(IMAGE));
    }
    let linked: usize = block
        .descendants_it()
        .filter(|node| node.is("a[href]"))
        .filter(|link| !link.ancestors_it(None).any(|up| up.is("a[href]")))
        .map(|link| length(&link.text()))
        .sum();
    if linked * 2 > all {
        return true;
    }
    block.is(LIST) && repeated(block, lists)
}

/// Whether the list `list`'s link targets, in order, are those of at least
/// two lists of the page: itself, and another it repeats.
fn repeated(list: &NodeRef<'_>, lists: &PageLists<'_>) -> bool {
    let own = targets(list, lists.base);
    !own.is_empty() && lists.targets.iter().filter(|other| **other == own).count() >= 2
}

/// The targets of `list`'s links, each resolved against `base` and read as
/// its path, query and fragment, so that an absolute target and a relative
/// one to the same page compare equal; a target that does not resolve is
/// read as written.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn targets(list: &NodeRef<'_>, base: Option<&Url>) -> Vec<String> {
    list.descendants_it()
        .filter(|node| node.is("a[href]"))
        .filter_map(|link| link.attr("href"))
        .map(|href| {
            base.and_then(|base| base.join(&href).ok()).map_or_else(
                || href.to_string(),
                |url| url[Position::BeforePath..].to_owned(),
            )
        })
        .collect()
}

/// The characters of `text`, white space aside.
fn length(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}
