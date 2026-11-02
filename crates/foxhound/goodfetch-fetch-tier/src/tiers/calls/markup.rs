//! An element's visibility as a static document's markup gives it (spec
//! #582's served-call table): hidden by the `hidden` attribute, by an
//! inline `display: none` on it or an ancestor, by the nearest inline
//! `visibility` that says `hidden` or `collapse`, by sitting in the
//! document's head, a `template`, a `script`, a `style` or a `noscript`,
//! or as an `<input type="hidden">`; visible otherwise, when it holds text
//! or an element the browser draws without text, in a document that holds
//! no `<style>` element and no stylesheet `<link>`. A stylesheet's rules
//! are not read, so an element such a document holds, which one of them
//! may hide, is left to the browser, as are an element with nothing to
//! draw, which the browser gives no size, and one inside a closed
//! `details`.

use dom_query::NodeRef;
use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};

/// What the markup says of the one element a selector matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    /// No element matches.
    Absent,
    /// It is visible.
    Visible,
    /// It is hidden.
    Hidden,
}

/// The elements that hide all they hold.
const HIDING: [&str; 7] = [
    "head", "template", "script", "style", "noscript", "title", "datalist",
];

/// The elements the browser draws without text of their own.
const DRAWN: [&str; 13] = [
    "img", "input", "svg", "video", "canvas", "iframe", "select", "textarea", "button", "object",
    "embed", "hr", "progress",
];

/// What the markup `html` says of the element the CSS selector `selector`
/// matches; `None` when it cannot tell: a selector the matcher does not
/// parse, a document with a declarative shadow root, more than one match,
/// whose wait the browser refuses as not strict, or an element whose
/// visibility needs layout or a stylesheet's rules.
pub(crate) fn shown(html: &str, selector: &str, work: &CountedWork) -> Option<Shown> {
    if html.to_ascii_lowercase().contains("shadowrootmode") {
        return None;
    }
    let matcher = work.compile_selector(Row::PageSelectorCompilations, selector)?;
    let document = work.parse_html(Row::PageParses, html);
    let selection = document.select_matcher(&matcher);
    let nodes = selection.nodes();
    let element = match nodes {
        [] => return Some(Shown::Absent),
        [element] => element,
        _ => return None,
    };
    let chain: Vec<NodeRef<'_>> = std::iter::once(*element)
        .chain(element.ancestors_it(None))
        .filter(NodeRef::is_element)
        .collect();
    let name = |node: &NodeRef<'_>| {
        node.node_name()
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default()
    };
    let hidden_input = name(element) == "input"
        && element
            .attr("type")
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("hidden"));
    let hidden = hidden_input
        || chain.iter().any(|node| {
            node.has_attr("hidden")
                || HIDING.contains(&name(node).as_str())
                || style(node, "display").as_deref() == Some("none")
        })
        || chain
            .iter()
            .find_map(|node| style(node, "visibility"))
            .is_some_and(|visibility| visibility == "hidden" || visibility == "collapse");
    if hidden {
        return Some(Shown::Hidden);
    }
    let in_closed_details = chain
        .iter()
        .skip(1)
        .any(|node| name(node) == "details" && !node.has_attr("open"));
    let draws = !element.text().trim().is_empty()
        || std::iter::once(*element)
            .chain(element.descendants())
            .any(|node| DRAWN.contains(&name(&node).as_str()));
    (draws && !in_closed_details && !styled(&document, work)).then_some(Shown::Visible)
}

/// Whether `document` holds a stylesheet: a `<style>` element, or a
/// `<link>` whose `rel` names `stylesheet`, an alternate one included.
fn styled(document: &ParsedHtml, work: &CountedWork) -> bool {
    !document
        .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "style"))
        .is_empty()
        || document
            .select_matcher(&work.literal_selector(Row::PageSelectorCompilations, "link[rel]"))
            .nodes()
            .iter()
            .any(|link| {
                link.attr("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|kind| kind.eq_ignore_ascii_case("stylesheet"))
                })
            })
}

/// The value of the property `property` in `node`'s inline `style`, lower
/// cased and without `!important`; the last declaration wins, as in CSS.
fn style(node: &NodeRef<'_>, property: &str) -> Option<String> {
    let style = node.attr("style")?;
    style
        .split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case(property))
        .map(|(_, value)| {
            value
                .trim()
                .to_ascii_lowercase()
                .trim_end_matches("!important")
                .trim()
                .to_owned()
        })
        .next_back()
}

#[cfg(test)]
mod tests;
