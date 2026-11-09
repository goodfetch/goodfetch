//! A document's tables, read from the page's tree (ADRs 0007 and 0138):
//! each table's caption, header rows and body rows, every cell its text
//! with white space collapsed, a row's header kept as its first cell, and
//! a table inside another listed on its own.

use dom_query::NodeRef;
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_protocol::page::Table;

use crate::whitespace::collapsed;

/// The tables of `tree`, in document order.
pub(crate) fn tables(tree: &ParsedHtml, work: &CountedWork) -> Vec<Table> {
    tree.select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "table"))
        .nodes()
        .iter()
        .map(table)
        .collect()
}

fn table(table: &NodeRef<'_>) -> Table {
    let mut caption = None;
    let (mut head, mut body, mut foot) = (Vec::new(), Vec::new(), Vec::new());
    for child in table.element_children() {
        match name(&child).as_str() {
            "caption" if caption.is_none() => caption = Some(text(&child)),
            "thead" => head.extend(rows(&child)),
            "tbody" => body.extend(rows(&child)),
            "tfoot" => foot.extend(rows(&child)),
            "tr" => body.push(child),
            _ => {}
        }
    }
    // Without a `<thead>`, the rows of `<th>` the table begins with head it.
    if head.is_empty() {
        let leading = body
            .iter()
            .take_while(|row| {
                let cells = cells(row);
                !cells.is_empty() && cells.iter().all(|cell| name(cell) == "th")
            })
            .count();
        head = body.drain(..leading).collect();
    }
    body.extend(foot);
    let read = |rows: Vec<NodeRef<'_>>| -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| cells(row).iter().map(text).collect())
            .collect()
    };
    Table {
        caption,
        header_rows: read(head),
        body_rows: read(body),
    }
}

/// The element's local name.
fn name(element: &NodeRef<'_>) -> String {
    element
        .node_name()
        .map(|name| name.to_string())
        .unwrap_or_default()
}

/// The rows of a table section.
fn rows<'a>(section: &NodeRef<'a>) -> Vec<NodeRef<'a>> {
    section
        .element_children()
        .into_iter()
        .filter(|child| name(child) == "tr")
        .collect()
}

/// The cells of a row.
fn cells<'a>(row: &NodeRef<'a>) -> Vec<NodeRef<'a>> {
    row.element_children()
        .into_iter()
        .filter(|child| matches!(name(child).as_str(), "td" | "th"))
        .collect()
}

/// The text of `element`, a table inside it left out and a `<br>` read as
/// a space, white space collapsed. Its descendants are walked by hand: a
/// page's tree can be deeper than a thread's stack allows a recursion.
fn text(element: &NodeRef<'_>) -> String {
    let mut text = String::new();
    let mut next: Vec<NodeRef<'_>> = element.children_it(true).collect();
    while let Some(node) = next.pop() {
        if node.is_text() {
            text.push_str(&node.text());
        } else if node.is_element() {
            match name(&node).as_str() {
                "table" => {}
                "br" => text.push(' '),
                _ => next.extend(node.children_it(true)),
            }
        }
    }
    collapsed(&text)
}
