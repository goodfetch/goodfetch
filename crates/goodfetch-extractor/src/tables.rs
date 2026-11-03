//! A document's tables, read from scraper's html5ever tree (ADR 0007):
//! each table's caption, header rows and body rows, every cell its text
//! with white space collapsed, a row's header kept as its first cell, and
//! a table inside another listed on its own.

use goodfetch_protocol::page::Table;
use scraper::{ElementRef, Html, Node, Selector};

/// The tables of `tree`, in document order.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(crate) fn tables(tree: &Html) -> Vec<Table> {
    let selector = Selector::parse("table").expect("invariant: `table` is a selector");
    tree.select(&selector).map(table).collect()
}

fn table(table: ElementRef<'_>) -> Table {
    let mut caption = None;
    let (mut head, mut body, mut foot) = (Vec::new(), Vec::new(), Vec::new());
    for child in table.child_elements() {
        match child.value().name() {
            "caption" if caption.is_none() => caption = Some(text(child)),
            "thead" => head.extend(rows(child)),
            "tbody" => body.extend(rows(child)),
            "tfoot" => foot.extend(rows(child)),
            "tr" => body.push(child),
            _ => {}
        }
    }
    // Without a `<thead>`, the rows of `<th>` the table begins with head it.
    if head.is_empty() {
        let leading = body
            .iter()
            .take_while(|row| {
                let mut cells = cells(**row).peekable();
                cells.peek().is_some() && cells.all(|cell| cell.value().name() == "th")
            })
            .count();
        head = body.drain(..leading).collect();
    }
    body.extend(foot);
    let read = |rows: Vec<ElementRef<'_>>| -> Vec<Vec<String>> {
        rows.into_iter()
            .map(|row| cells(row).map(text).collect())
            .collect()
    };
    Table {
        caption,
        header_rows: read(head),
        body_rows: read(body),
    }
}

/// The rows of a table section.
fn rows(section: ElementRef<'_>) -> impl Iterator<Item = ElementRef<'_>> {
    section
        .child_elements()
        .filter(|child| child.value().name() == "tr")
}

/// The cells of a row.
fn cells(row: ElementRef<'_>) -> impl Iterator<Item = ElementRef<'_>> {
    row.child_elements()
        .filter(|child| matches!(child.value().name(), "td" | "th"))
}

/// The text of `element`, a table inside it left out and a `<br>` read as
/// a space, white space collapsed.
fn text(element: ElementRef<'_>) -> String {
    let mut text = String::new();
    let mut descendants = element.descendants();
    // The element itself first, then its descendants.
    descendants.next();
    let mut skip_under = None;
    for node in descendants {
        if let Some(table) = skip_under {
            if node.ancestors().any(|ancestor| ancestor.id() == table) {
                continue;
            }
            skip_under = None;
        }
        match node.value() {
            Node::Text(words) => text.push_str(words),
            Node::Element(child) if child.name() == "table" => skip_under = Some(node.id()),
            Node::Element(child) if child.name() == "br" => text.push(' '),
            _ => {}
        }
    }
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
