//! A document read as blocks of text, on its tree (ADR 0138): the main
//! content's text a paragraph per block, and the main content "fit"
//! (`Crawl4AI`'s meaning, spec #580): without its link-dense blocks, those
//! with more than half their text in links, its image-only blocks and the
//! navigation lists the document repeats, each link compared by its target
//! resolved against the document's base URL. Lengths are counted in
//! characters, white space aside.

use dom_query::{Matcher, NodeRef};
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use url::{Position, Url};

use crate::extractor::Article;

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

/// The text of `nodes`, in order, each block's text a paragraph, its white
/// space made one space, the paragraphs one blank line apart.
pub(crate) fn text<'a>(nodes: impl IntoIterator<Item = NodeRef<'a>>) -> String {
    let mut blocks = Blocks::default();
    for node in nodes {
        blocks.read(&node);
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

/// The lists of a page, each as its links' targets, and the base URL they
/// were resolved against: what "fit" compares the main content's lists to,
/// read from the page's tree before the main-content step consumes it.
pub(crate) struct PageLists {
    base: Option<Url>,
    targets: Vec<Vec<String>>,
}

/// The matchers "fit" reads with, compiled once for the engine.
struct Matchers {
    candidate: Matcher,
    image: Matcher,
    list: Matcher,
    link: Matcher,
}

impl Matchers {
    fn of(work: &CountedWork) -> Self {
        let compiled = |selector| work.literal_selector(Row::PageSelectorCompilations, selector);
        Self {
            candidate: compiled(CANDIDATE),
            image: compiled(IMAGE),
            list: compiled(LIST),
            link: compiled(LINK),
        }
    }
}

/// A link with a target.
const LINK: &str = "a[href]";

impl PageLists {
    /// The lists of `page`, the tree of the whole page, whose base URL is
    /// `base_url`.
    pub(crate) fn of(page: &ParsedHtml, base_url: &str, work: &CountedWork) -> Self {
        let base = Url::parse(base_url).ok();
        let matchers = Matchers::of(work);
        let targets = page
            .select_matcher(&matchers.list)
            .nodes()
            .iter()
            .map(|list| targets(list, base.as_ref(), &matchers))
            .filter(|targets| !targets.is_empty())
            .collect();
        Self { base, targets }
    }
}

/// Takes out of `article`, the main content, its link-dense blocks, its
/// image-only blocks and the lists whose links another list of the page,
/// `lists`, repeats, in order; whether anything of it is left.
pub(crate) fn fit(article: &Article, lists: &PageLists, work: &CountedWork) -> bool {
    let matchers = Matchers::of(work);
    let root = article.root();
    if article.whole && root.is_match(&matchers.candidate) && left_out(&root, lists, &matchers) {
        return false;
    }
    prune(&root, lists, &matchers);
    true
}

/// Takes out of `node`'s descendants, from the outermost, each candidate
/// block "fit" leaves out.
fn prune(node: &NodeRef<'_>, lists: &PageLists, matchers: &Matchers) {
    for child in node.element_children() {
        if child.is_match(&matchers.candidate) && left_out(&child, lists, matchers) {
            child.remove_from_parent();
        } else {
            prune(&child, lists, matchers);
        }
    }
}

fn left_out(block: &NodeRef<'_>, lists: &PageLists, matchers: &Matchers) -> bool {
    let all = length(&block.text());
    if all == 0 {
        return block
            .descendants_it()
            .any(|node| node.is_match(&matchers.image));
    }
    let linked: usize = block
        .descendants_it()
        .filter(|node| node.is_match(&matchers.link))
        .filter(|link| {
            !link
                .ancestors_it(None)
                .any(|up| up.is_match(&matchers.link))
        })
        .map(|link| length(&link.text()))
        .sum();
    if linked * 2 > all {
        return true;
    }
    block.is_match(&matchers.list) && repeated(block, lists, matchers)
}

/// Whether the list `list`'s link targets, in order, are those of at least
/// two lists of the page: itself, and another it repeats.
fn repeated(list: &NodeRef<'_>, lists: &PageLists, matchers: &Matchers) -> bool {
    let own = targets(list, lists.base.as_ref(), matchers);
    !own.is_empty() && lists.targets.iter().filter(|other| **other == own).count() >= 2
}

/// The targets of `list`'s links, each resolved against `base` and read as
/// its path, query and fragment, so that an absolute target and a relative
/// one to the same page compare equal; a target that does not resolve is
/// read as written.
fn targets(list: &NodeRef<'_>, base: Option<&Url>, matchers: &Matchers) -> Vec<String> {
    list.descendants_it()
        .filter(|node| node.is_match(&matchers.link))
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
