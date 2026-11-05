//! A document's structured data, read from the page's tree (ADRs 0007 and
//! 0138): each JSON-LD block that parses, as JSON, and its microdata
//! items as the HTML Standard's microdata and their JSON conversion read
//! them, `itemref` included and an item that would hold itself written
//! `"ERROR"` where it loops, as the conversion writes it. One call's
//! microdata does a bounded amount of work, whatever the page's items
//! share: past its budget, a value is written `"ERROR"` too.

use std::collections::{BTreeMap, HashMap, HashSet};

use dom_query::{NodeId, NodeRef};
use goodfetch_counted_work::ParsedHtml;
use goodfetch_protocol::page::{MicrodataItem, MicrodataValue};
use serde_json::Value;
use url::Url;

/// Every `<script type="application/ld+json">` block of `tree` that
/// parses, in document order, its text as written.
pub(crate) fn json_ld(tree: &ParsedHtml) -> Vec<Value> {
    tree.root()
        .descendants_it()
        .filter(|script| script.has_name("script"))
        .filter(|script| {
            script.attr("type").is_some_and(|kind| {
                kind.trim_matches(|character: char| character.is_ascii_whitespace())
                    .eq_ignore_ascii_case("application/ld+json")
            })
        })
        .filter_map(|script| serde_json::from_str(&script.text()).ok())
        .collect()
}

/// The top-level microdata items of `tree`, in tree order: the elements
/// with `itemscope` and no `itemprop`, their URLs resolved against
/// `base_url`. Past the budget of the whole call, each value not read yet
/// is `"ERROR"`, and an item not reached yet has no properties.
pub(crate) fn microdata(tree: &ParsedHtml, base_url: &str) -> Vec<MicrodataItem> {
    let document = Document::of(tree, base_url);
    let elements: Vec<NodeRef<'_>> = tree
        .root()
        .descendants_it()
        .filter(|element| {
            element.is_element() && element.has_attr("itemscope") && !element.has_attr("itemprop")
        })
        .collect();
    let mut reading = Reading {
        memory: Vec::new(),
        left: Budget::WHOLE,
    };
    elements
        .into_iter()
        .map(|element| document.item(element, &mut reading))
        .collect()
}

/// How many items deep a value is read: an item deeper is written
/// `"ERROR"` as a loop is, so a hostile document cannot exhaust the stack.
const MAX_DEPTH: usize = 128;

/// What one call's reading may still do. Items that reach one container
/// through `itemref` each read it again, as the JSON conversion does, so
/// a page of a few kilobytes holds exponentially many items (finding
/// 580-1): the budget bounds the call's time and its answer's size, as the
/// depth bounds its stack.
#[derive(Clone, Copy)]
struct Budget {
    /// Each element a crawl visits, each node a value's text is read from,
    /// and each value written, a copy's values included, is a step.
    steps: usize,
    /// The bytes of the text values written, a copy's included.
    text: usize,
}

impl Budget {
    /// A whole call's: far more than any page's own microdata takes.
    const WHOLE: Self = Self {
        steps: 100_000,
        text: 4 << 20,
    };

    /// Takes `steps` and `text` bytes, when both are left; otherwise spends
    /// what is left, so that the call reads nothing more.
    fn take(&mut self, steps: usize, text: usize) -> bool {
        if steps <= self.steps && text <= self.text {
            self.steps -= steps;
            self.text -= text;
            true
        } else {
            *self = Self { steps: 0, text: 0 };
            false
        }
    }

    fn spent(self) -> bool {
        self.steps == 0
    }
}

/// Where one call's reading stands: the items being read around the
/// current one, outermost first, and what is left of its budget.
struct Reading {
    memory: Vec<NodeId>,
    left: Budget,
}

/// What reading the items needs of the whole document.
struct Document<'a> {
    base: Option<Url>,
    /// Each element's place in tree order.
    order: HashMap<NodeId, usize>,
    /// The first element of each ID.
    ids: HashMap<String, NodeRef<'a>>,
}

impl<'a> Document<'a> {
    fn of(tree: &'a ParsedHtml, base_url: &str) -> Self {
        let mut order = HashMap::new();
        let mut ids = HashMap::new();
        for (place, node) in tree.root().descendants_it().enumerate() {
            order.insert(node.id, place);
            if node.is_element()
                && let Some(id) = node.attr("id")
            {
                ids.entry(id.to_string()).or_insert(node);
            }
        }
        Self {
            base: Url::parse(base_url).ok(),
            order,
            ids,
        }
    }

    /// The item `element` is.
    fn item(&self, element: NodeRef<'a>, reading: &mut Reading) -> MicrodataItem {
        reading.memory.push(element.id);
        let attribute = |name| element.attr(name);
        let mut properties: BTreeMap<String, Vec<MicrodataValue>> = BTreeMap::new();
        for property in self.properties(element, &mut reading.left) {
            let mut value = self.property_value(property, reading);
            let names = tokens(property.attr("itemprop").as_deref());
            let last = names.len().saturating_sub(1);
            for (place, name) in names.into_iter().enumerate() {
                // Each name past the first holds a copy of the value, which
                // costs what the value did.
                if place > 0
                    && let Some(read) = &value
                {
                    let (steps, text) = weight(read);
                    if !reading.left.take(steps, text) {
                        value = None;
                    }
                }
                // The last name takes the value itself: a copy for it would
                // copy the item's whole subtree again at each level above
                // it, work no step pays for.
                let written = if place == last {
                    value.take()
                } else {
                    value.clone()
                };
                properties
                    .entry(name)
                    .or_default()
                    .push(written.unwrap_or_else(error));
            }
        }
        reading.memory.pop();
        MicrodataItem {
            types: tokens(attribute("itemtype").as_deref()),
            id: attribute("itemid").and_then(|id| self.resolve(&id)),
            properties,
        }
    }

    /// The value of the property `property`; none past the budget.
    fn property_value(
        &self,
        property: NodeRef<'a>,
        reading: &mut Reading,
    ) -> Option<MicrodataValue> {
        if reading.left.spent() {
            return None;
        }
        if !property.has_attr("itemscope") {
            return self
                .value(property, &mut reading.left)
                .map(MicrodataValue::Text);
        }
        if reading.memory.len() >= MAX_DEPTH || reading.memory.contains(&property.id) {
            return Some(error());
        }
        reading
            .left
            .take(1, 0)
            .then(|| MicrodataValue::Item(self.item(property, reading)))
    }

    /// The properties of the item `root` is, in tree order: the HTML
    /// Standard's crawl over its children and the elements its `itemref`
    /// names, which stops at each item it meets, and at the budget's end.
    fn properties(&self, root: NodeRef<'a>, left: &mut Budget) -> Vec<NodeRef<'a>> {
        if left.spent() {
            return Vec::new();
        }
        let mut pending: Vec<NodeRef<'a>> = root.element_children();
        pending.extend(
            tokens(root.attr("itemref").as_deref())
                .iter()
                .filter_map(|id| self.ids.get(id.as_str()).copied()),
        );
        let mut seen = HashSet::from([root.id]);
        let mut results = Vec::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current.id) {
                continue;
            }
            if !left.take(1, 0) {
                break;
            }
            if !current.has_attr("itemscope") {
                pending.extend(current.element_children());
            }
            if current
                .attr("itemprop")
                .is_some_and(|names| names.split_ascii_whitespace().next().is_some())
            {
                results.push(current);
            }
        }
        results.sort_by_key(|element| self.order.get(&element.id).copied());
        results
    }

    /// The value of the property `element` is, as the HTML Standard reads
    /// each element; none past the budget.
    fn value(&self, element: NodeRef<'a>, left: &mut Budget) -> Option<String> {
        let attribute = |name| element.attr(name).map(|value| value.to_string());
        let url = |name| {
            attribute(name)
                .and_then(|value| self.resolve(&value))
                .unwrap_or_default()
        };
        let name = element
            .node_name()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let value = match name.as_str() {
            "meta" => attribute("content").unwrap_or_default(),
            "audio" | "embed" | "iframe" | "img" | "source" | "track" | "video" => url("src"),
            "a" | "area" | "link" => url("href"),
            "object" => url("data"),
            "data" | "meter" => attribute("value").unwrap_or_default(),
            "time" if attribute("datetime").is_some() => attribute("datetime").unwrap_or_default(),
            _ => return text(&element, left),
        };
        left.take(1, value.len()).then_some(value)
    }

    /// `value` resolved against the base URL; `None` when either does not
    /// parse.
    fn resolve(&self, value: &str) -> Option<String> {
        self.base
            .as_ref()
            .and_then(|base| base.join(value).ok())
            .map(String::from)
    }
}

/// The text of `element`, each node it is read from, itself first, a step
/// of the budget; none past it.
fn text(element: &NodeRef<'_>, left: &mut Budget) -> Option<String> {
    let mut text = String::new();
    for node in std::iter::once(*element).chain(element.descendants_it()) {
        let piece = if node.is_text() {
            node.text().to_string()
        } else {
            String::new()
        };
        if !left.take(1, piece.len()) {
            return None;
        }
        text.push_str(&piece);
    }
    Some(text)
}

/// The steps and text bytes a copy of `value` costs.
fn weight(value: &MicrodataValue) -> (usize, usize) {
    match value {
        MicrodataValue::Text(text) => (1, text.len()),
        MicrodataValue::Item(item) => item
            .properties
            .values()
            .flatten()
            .map(weight)
            .fold((1, 0), |(steps, text), (more_steps, more_text)| {
                (steps + more_steps, text + more_text)
            }),
    }
}

/// The value the JSON conversion writes where it reads no further.
fn error() -> MicrodataValue {
    MicrodataValue::Text("ERROR".to_owned())
}

/// The unique tokens of an attribute's value, in order; none when it is
/// absent.
fn tokens(value: Option<&str>) -> Vec<String> {
    let mut seen = HashSet::new();
    value
        .unwrap_or_default()
        .split_ascii_whitespace()
        .filter(|token| seen.insert(*token))
        .map(ToOwned::to_owned)
        .collect()
}
