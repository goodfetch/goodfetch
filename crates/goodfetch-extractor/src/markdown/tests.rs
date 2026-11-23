//! The Markdown writer's bound on depth: a page nested deeper than a
//! thread's stack holds is written on the engine's stack, a page nested up
//! to the bound is written as it stands, and what lies past it is written
//! flattened, its text kept.
#![expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a test's own parse, on no Shared path"
)]

use goodfetch_counted_work::{CountedWork, Row};

use super::{MAX_DEPTH, markdown};

/// The stack of the engine's threads: std's default for a thread it starts,
/// as the CPU pool's that run the Page analysis are, and Tokio's for its
/// own; the engine sets neither.
const ENGINE_STACK: usize = 2 * 1024 * 1024;

/// The Markdown of `html`'s body.
fn written(html: &str) -> String {
    let page = CountedWork::default().parse_html(Row::PageParses, html);
    markdown(page.body().unwrap().children_it(false))
}

/// `inner` in `depth` nested `<div>` elements, as a page's body.
fn nested(depth: usize, inner: &str) -> String {
    format!(
        "<body>{}{inner}{}</body>",
        "<div>".repeat(depth),
        "</div>".repeat(depth)
    )
}

/// Covers: story:6-1
#[test]
fn a_page_nested_deeper_than_a_stack_holds_is_written_on_the_engines_stack() {
    #[expect(
        clippy::disallowed_methods,
        reason = "only a thread of the test's own has the engine's stack size; it ends before the test does"
    )]
    let written = std::thread::Builder::new()
        .stack_size(ENGINE_STACK)
        .spawn(|| {
            // The tree is built by hand: a parse of 100,000 nested elements
            // takes minutes, as html5ever scans its stack of open elements
            // for each start tag.
            let page = CountedWork::default().parse_html(Row::PageParses, "<body></body>");
            let body = page.body().unwrap();
            let mut deepest = body;
            for _ in 0..100_000 {
                let div = page.tree.new_element("div");
                deepest.append_child(&div);
                deepest = div;
            }
            deepest.append_child(&page.tree.new_text("The deepest line."));
            markdown(body.children_it(false))
        })
        .unwrap()
        .join()
        .unwrap();

    assert_eq!(written, "The deepest line.\n");
}

/// Covers: story:6-1
#[test]
fn a_page_nested_to_the_bound_is_written_as_it_stands_and_one_level_past_it_flattened() {
    // The `<p>` and the `<em>` take two levels and the emphasized text a
    // third: at `MAX_DEPTH - 2` the `<em>` sits at the bound and holds its
    // text, and one level deeper the `<p>` does, the `<em>` emptied beside it.
    let line = "<p>A line with <em>stress</em>.</p>";

    assert_eq!(
        written(&nested(MAX_DEPTH - 2, line)),
        "A line with *stress*.\n"
    );
    assert_eq!(
        written(&nested(MAX_DEPTH - 1, line)),
        "A line with stress.\n"
    );
}

/// Covers: story:6-1
#[test]
fn a_table_cut_by_the_bound_keeps_its_cells_text() {
    // The table, its row group, its row and its cells take four levels: at
    // `MAX_DEPTH - 4` a cell sits at the bound and holds its text, and one
    // level deeper the row does, which writes only cells, so what lies below
    // it is written after the table.
    let table = "<table><tr><th>Power</th><td>8 W</td></tr></table><p>After the table.</p>";

    assert_eq!(
        written(&nested(MAX_DEPTH - 4, table)),
        "| Power | 8 W |\n| --- | --- |\n\nAfter the table.\n"
    );
    assert_eq!(
        written(&nested(MAX_DEPTH - 3, table)),
        "Power\n\n8 W\n\nAfter the table.\n"
    );
}
