//! The Markdown converter (ADR 0007): htmd writes HTML as Markdown with
//! GFM's tables, ATX headings, `-` bullets and fenced code. Two of its
//! handlers are this module's, the two fixes ADR 0007 names until htmd
//! takes them upstream: code in a `<pre>` without a `<code>` element is a
//! code block, where htmd writes a paragraph, and a table whose rows start
//! with a row header (`<th>` beside `<td>`) keeps every cell, where htmd
//! drops each row's `<td>`s or its `<th>`s. A third keeps a heading
//! with a `<br>` whole, where htmd cuts it in two.
//!
//! The converter is built once, and reads the page's own tree (ADR 0138):
//! the nodes to write are walked into the html5ever tree `markup5ever_rcdom`
//! holds, which htmd's non-parsing entry reads, so no HTML is serialized or
//! parsed for it.

use std::rc::Rc;
use std::sync::LazyLock;

use dom_query::{NodeData as PageData, NodeRef};
use htmd::element_handler::{HandlerResult, Handlers};
use htmd::options::{BulletListMarker, Options};
use htmd::{Element, HtmlToMarkdown};
use markup5ever_htmd::tendril::StrTendril;
use markup5ever_htmd::{Attribute, QualName, local_name, ns};
use markup5ever_rcdom::{Node, NodeData};

#[cfg(test)]
mod tests;

/// How deep the copy nests a node below the body: htmd's walker and this
/// module's handlers recurse once a level or more, and a page nests as deep
/// as it likes. Browsers' parsers stop at 512 levels (Blink's and WebKit's
/// `maximumHTMLParserDOMTreeDepth`) and attach what lies deeper to the
/// deepest node's parent, as the copy does; half that leaves room on a
/// 2 MiB stack, which unoptimized htmd fills near 700 levels.
const MAX_DEPTH: usize = 256;

/// The converter, built once for the engine.
static CONVERTER: LazyLock<HtmlToMarkdown> = LazyLock::new(|| {
    HtmlToMarkdown::builder()
        .options(Options {
            bullet_list_marker: BulletListMarker::Dash,
            ul_bullet_spacing: 1,
            ol_number_spacing: 1,
            ..Options::default()
        })
        .add_handler(vec!["h1", "h2", "h3", "h4", "h5", "h6"], heading)
        .add_handler(vec!["pre"], pre)
        .add_handler(vec!["table"], table)
        .build()
});

/// The nodes `nodes` of a page's tree, in order, as Markdown, ending with
/// one newline; empty when they hold no text. They are written as the body
/// of a document of their own, as a parse of their HTML would hold them.
pub(crate) fn markdown<'a>(nodes: impl IntoIterator<Item = NodeRef<'a>>) -> String {
    let body = element(
        QualName::new(None, ns!(html), local_name!("body")),
        Vec::new(),
    );
    for node in nodes {
        copy(node, &body);
    }
    let html = element(
        QualName::new(None, ns!(html), local_name!("html")),
        Vec::new(),
    );
    append(
        &html,
        element(
            QualName::new(None, ns!(html), local_name!("head")),
            Vec::new(),
        ),
    );
    append(&html, body);
    let document = Node::new(NodeData::Document);
    append(&document, html);
    let written = CONVERTER.tree_to_markdown(&document);
    let written = written.trim_matches('\n');
    if written.is_empty() {
        String::new()
    } else {
        format!("{written}\n")
    }
}

/// An element of `markup5ever_rcdom`'s tree, with no children yet.
fn element(name: QualName, attrs: Vec<Attribute>) -> Rc<Node> {
    Node::new(NodeData::Element {
        name,
        attrs: attrs.into(),
        template_contents: None.into(),
        mathml_annotation_xml_integration_point: false,
    })
}

/// Appends `child` to `parent`'s children, its last.
fn append(parent: &Rc<Node>, child: Rc<Node>) {
    child.parent.set(Some(Rc::downgrade(parent)));
    parent.children.borrow_mut().push(child);
}

/// Appends to `parent` the copy of `node` and of its descendants, by hand
/// rather than by recursion, as a page's tree can be deeper than a thread's
/// stack allows one. A node is nested at most [`MAX_DEPTH`] levels below
/// `parent`: the children of a node at that depth are its siblings, so what
/// lies deeper is written flattened, in document order. Text beside text is
/// one text node, as a parse makes it; a comment is kept, and what htmd
/// never reads (a doctype, a template's contents, which are no child of it)
/// is not. Texts and attribute values are copied, as the page's tendrils are
/// atomic and rcdom's are not.
fn copy(node: NodeRef<'_>, parent: &Rc<Node>) {
    // Each node to copy, the copy it goes under, and its own depth.
    let mut next = vec![(node, Rc::clone(parent), 1)];
    while let Some((node, parent, depth)) = next.pop() {
        let copied = node.query(|tree_node| match &tree_node.data {
            PageData::Text { contents } => {
                let children = parent.children.borrow();
                if let Some(NodeData::Text { contents: last }) =
                    children.last().map(|last| &last.data)
                {
                    last.borrow_mut().push_slice(contents);
                    return None;
                }
                drop(children);
                Some(Node::new(NodeData::Text {
                    contents: StrTendril::from_slice(contents).into(),
                }))
            }
            PageData::Comment { contents } => Some(Node::new(NodeData::Comment {
                contents: StrTendril::from_slice(contents),
            })),
            PageData::Element(page_element) => Some(element(
                QualName::new(
                    page_element.name.prefix.clone(),
                    page_element.name.ns.clone(),
                    page_element.name.local.clone(),
                ),
                page_element
                    .attrs
                    .iter()
                    .map(|attribute| Attribute {
                        name: QualName::new(
                            attribute.name.prefix.clone(),
                            attribute.name.ns.clone(),
                            attribute.name.local.clone(),
                        ),
                        value: StrTendril::from_slice(&attribute.value),
                    })
                    .collect(),
            )),
            _ => None,
        });
        let Some(copied) = copied.flatten() else {
            continue;
        };
        append(&parent, Rc::clone(&copied));
        if node.is_element() {
            let (under, depth) = if depth < MAX_DEPTH {
                (copied, depth + 1)
            } else {
                (parent, depth)
            };
            next.extend(
                node.children_it(true)
                    .map(|child| (child, Rc::clone(&under), depth)),
            );
        }
    }
}

/// A heading on one line, as an ATX heading must be: htmd's own, each
/// line break it wrote for a `<br>` a space.
fn heading(handlers: &dyn Handlers, element: Element<'_>) -> Option<HandlerResult> {
    let written = handlers.fallback(element)?;
    let line = written
        .content
        .trim_matches('\n')
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join(" ");
    Some(HandlerResult {
        content: format!("\n\n{line}\n\n"),
        markdown_translated: written.markdown_translated,
    })
}

/// A `<pre>`: htmd's own handler when it holds one `<code>` element, else a
/// fenced code block of its text as it stands.
fn pre(handlers: &dyn Handlers, element: Element<'_>) -> Option<HandlerResult> {
    let children = element.node.children.borrow();
    let one_code = children.len() == 1 && tag_of(&children[0]) == Some("code");
    drop(children);
    if one_code {
        return handlers.fallback(element);
    }
    let mut text = String::new();
    text_of(element.node, &mut text);
    let text = text.trim_end_matches('\n');
    if text.is_empty() {
        return None;
    }
    let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
    Some(format!("\n\n{fence}\n{text}\n{fence}\n\n").into())
}

/// A `<table>` with headers: every row's cells in order, row headers
/// included, its first row (its `<thead>`'s when it has one) as the header
/// row GFM requires. A table with no header cell is a layout table, and a
/// table inside a cell cannot be a GFM table: both are htmd's own.
fn table(handlers: &dyn Handlers, element: Element<'_>) -> Option<HandlerResult> {
    if !has_header_cell(element.node) || inside_cell(element.node) {
        return handlers.fallback(element);
    }
    let mut caption = None;
    let mut head = Vec::new();
    let mut body = Vec::new();
    for child in element.node.children.borrow().iter() {
        match tag_of(child) {
            Some("caption") => {
                caption = Some(cell_text(handlers, child));
            }
            Some("thead") => rows_of(handlers, child, &mut head),
            Some("tbody" | "tfoot") => rows_of(handlers, child, &mut body),
            Some("tr") => body.push(cells_of(handlers, child)),
            _ => {}
        }
    }
    let mut rows = head.into_iter().chain(body).filter(|row| !row.is_empty());
    let header = rows.next()?;
    let rows: Vec<Vec<String>> = rows.collect();
    let columns = rows.iter().map(Vec::len).fold(header.len(), usize::max);
    let mut written = String::from("\n\n");
    if let Some(caption) = caption.filter(|caption| !caption.is_empty()) {
        written.push_str(&caption);
        written.push_str("\n\n");
    }
    written.push_str(&row_line(&header, columns));
    written.push_str(&row_line(&vec!["---".to_owned(); columns], columns));
    for row in &rows {
        written.push_str(&row_line(row, columns));
    }
    written.push('\n');
    Some(written.into())
}

fn rows_of(handlers: &dyn Handlers, group: &Rc<Node>, rows: &mut Vec<Vec<String>>) {
    for row in group.children.borrow().iter() {
        if tag_of(row) == Some("tr") {
            rows.push(cells_of(handlers, row));
        }
    }
}

/// A row's cells, header cells and data cells alike, in order.
fn cells_of(handlers: &dyn Handlers, row: &Rc<Node>) -> Vec<String> {
    row.children
        .borrow()
        .iter()
        .filter(|cell| matches!(tag_of(cell), Some("th" | "td")))
        .map(|cell| cell_text(handlers, cell))
        .collect()
}

/// A cell's content as one line of Markdown, its pipes escaped.
fn cell_text(handlers: &dyn Handlers, cell: &Rc<Node>) -> String {
    let content = handlers.walk_children(cell).content;
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn row_line(cells: &[String], columns: usize) -> String {
    let mut line = String::from("|");
    for column in 0..columns {
        line.push(' ');
        line.push_str(cells.get(column).map_or("", String::as_str));
        line.push_str(" |");
    }
    line.push('\n');
    line
}

/// Whether `table` has a `<th>` or a `<thead>` of its own, outside any
/// table nested in it.
fn has_header_cell(table: &Rc<Node>) -> bool {
    table
        .children
        .borrow()
        .iter()
        .any(|child| match tag_of(child) {
            Some("thead" | "th") => true,
            Some("table") => false,
            _ => has_header_cell(child),
        })
}

fn inside_cell(node: &Rc<Node>) -> bool {
    let mut parent = parent_of(node);
    while let Some(node) = parent {
        if matches!(tag_of(&node), Some("td" | "th")) {
            return true;
        }
        parent = parent_of(&node);
    }
    false
}

fn parent_of(node: &Rc<Node>) -> Option<Rc<Node>> {
    let parent = node.parent.take();
    let upgraded = parent.as_ref().and_then(std::rc::Weak::upgrade);
    node.parent.set(parent);
    upgraded
}

fn tag_of(node: &Rc<Node>) -> Option<&str> {
    match &node.data {
        NodeData::Element { name, .. } => Some(name.local.as_ref()),
        _ => None,
    }
}

/// Every text of `node`, in document order, as it stands.
fn text_of(node: &Rc<Node>, text: &mut String) {
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => text.push_str(&contents.borrow()),
            NodeData::Element { .. } => text_of(child, text),
            _ => {}
        }
    }
}

/// The longest run of `symbol` in `text`.
fn longest_run(text: &str, symbol: char) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for character in text.chars() {
        if character == symbol {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}
