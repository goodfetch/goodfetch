//! The Markdown converter (ADR 0007): htmd writes HTML as Markdown with
//! GFM's tables, ATX headings, `-` bullets and fenced code. Two of its
//! handlers are this module's, the two fixes ADR 0007 names until htmd
//! takes them upstream: code in a `<pre>` without a `<code>` element is a
//! code block, where htmd writes a paragraph, and a table whose rows start
//! with a row header (`<th>` beside `<td>`) keeps every cell, where htmd
//! drops each row's `<td>`s or its `<th>`s. A third keeps a heading
//! with a `<br>` whole, where htmd cuts it in two.

use std::rc::Rc;

use htmd::element_handler::{HandlerResult, Handlers};
use htmd::options::{BulletListMarker, Options};
use htmd::{Element, HtmlToMarkdown};
use markup5ever_rcdom::{Node, NodeData};

/// `html` as Markdown, ending with one newline; empty for HTML with no
/// text.
pub(crate) fn markdown(html: &str) -> String {
    let converter = HtmlToMarkdown::builder()
        .options(Options {
            bullet_list_marker: BulletListMarker::Dash,
            ul_bullet_spacing: 1,
            ol_number_spacing: 1,
            ..Options::default()
        })
        .add_handler(vec!["h1", "h2", "h3", "h4", "h5", "h6"], heading)
        .add_handler(vec!["pre"], pre)
        .add_handler(vec!["table"], table)
        .build();
    let written = converter
        .convert(html)
        .expect("invariant: htmd reads from a string, which never fails");
    let written = written.trim_matches('\n');
    if written.is_empty() {
        String::new()
    } else {
        format!("{written}\n")
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
