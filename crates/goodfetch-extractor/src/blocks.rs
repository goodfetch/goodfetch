//! A document read as blocks of text: its body without its head, the main
//! content's text a paragraph per block, and the main content "fit"
//! (`Crawl4AI`'s meaning, spec #580): without its link-dense blocks, those
//! with more than half their text in links, its image-only blocks and the
//! navigation lists the document repeats. Lengths are counted in
//! characters, white space aside.

use dom_query::{Document, NodeRef};

/// The elements that hold no text a person reads.
const NOT_TEXT: [&str; 6] = ["head", "script", "style", "noscript", "template", "title"];

/// The elements that run inside a block, which every other element is.
const INLINE: [&str; 30] = [
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "del", "dfn", "em", "font", "i", "img",
    "ins", "kbd", "label", "mark", "q", "s", "samp", "small", "span", "strong", "sub", "sup",
    "time", "u", "var", "wbr",
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
pub(crate) fn body(html: &str) -> String {
    let document = Document::from(html);
    document.select("body").inner_html().to_string()
}

/// The text of `html`, each block's text a paragraph, its white space made
/// one space, the paragraphs one blank line apart.
pub(crate) fn text(html: &str) -> String {
    let document = Document::from(html);
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
        let block = !INLINE.contains(&name);
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
/// (the whole page) repeats, in order.
pub(crate) fn fit(main: &str, document: &str) -> String {
    let page = Document::from(document);
    let page_lists: Vec<Vec<String>> = page
        .select(LIST)
        .nodes()
        .iter()
        .map(targets)
        .filter(|targets| !targets.is_empty())
        .collect();
    let content = Document::from(main);
    let root = content.select("body");
    for node in root.nodes() {
        prune(node, &page_lists);
    }
    root.inner_html().to_string()
}

/// Takes out of `node`'s descendants, from the outermost, each candidate
/// block "fit" leaves out.
fn prune(node: &NodeRef<'_>, page_lists: &[Vec<String>]) {
    for child in node.element_children() {
        if child.is(CANDIDATE) && left_out(&child, page_lists) {
            child.remove_from_parent();
        } else {
            prune(&child, page_lists);
        }
    }
}

fn left_out(block: &NodeRef<'_>, page_lists: &[Vec<String>]) -> bool {
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
    block.is(LIST) && repeated(block, page_lists)
}

/// Whether the list `list`'s link targets, in order, are those of at least
/// two lists of the page: itself, and another it repeats.
fn repeated(list: &NodeRef<'_>, page_lists: &[Vec<String>]) -> bool {
    let own = targets(list);
    !own.is_empty() && page_lists.iter().filter(|other| **other == own).count() >= 2
}

/// The targets of `list`'s links, each as its path, so that an absolute
/// target and a relative one to the same page compare equal.
fn targets(list: &NodeRef<'_>) -> Vec<String> {
    list.descendants_it()
        .filter(|node| node.is("a[href]"))
        .filter_map(|link| link.attr("href"))
        .map(|href| path_of(&href).to_owned())
        .collect()
}

/// The path, query and fragment of `href`, or `href` when it has no
/// scheme and host.
fn path_of(href: &str) -> &str {
    let Some(after) = href.find("://").map(|at| at + "://".len()) else {
        return href;
    };
    href[after..]
        .find('/')
        .map_or("/", |at| &href[after + at..])
}

/// The characters of `text`, white space aside.
fn length(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}
