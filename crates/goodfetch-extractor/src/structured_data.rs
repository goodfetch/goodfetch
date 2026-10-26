//! A document's structured data, read from scraper's html5ever tree
//! (ADR 0007): each JSON-LD block that parses, as JSON, and its microdata
//! items as the HTML Standard's microdata and their JSON conversion read
//! them, `itemref` included and an item that would hold itself written
//! `"ERROR"` where it loops, as the conversion writes it.

use std::collections::{BTreeMap, HashMap, HashSet};

use ego_tree::NodeId;
use goodfetch_protocol::page::{MicrodataItem, MicrodataValue};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use url::Url;

/// Every `<script type="application/ld+json">` block of `tree` that
/// parses, in document order, its text as written.
pub(crate) fn json_ld(tree: &Html) -> Vec<Value> {
    let selector = Selector::parse("script[type]").expect("invariant: the selector is CSS");
    tree.select(&selector)
        .filter(|script| {
            script.value().attr("type").is_some_and(|kind| {
                kind.trim_matches(|character: char| character.is_ascii_whitespace())
                    .eq_ignore_ascii_case("application/ld+json")
            })
        })
        .filter_map(|script| serde_json::from_str(&script.text().collect::<String>()).ok())
        .collect()
}

/// The top-level microdata items of `tree`, in tree order: the elements
/// with `itemscope` and no `itemprop`, their URLs resolved against
/// `base_url`.
pub(crate) fn microdata(tree: &Html, base_url: &str) -> Vec<MicrodataItem> {
    let document = Document::of(tree, base_url);
    let elements: Vec<ElementRef<'_>> = tree
        .tree
        .root()
        .descendants()
        .filter_map(ElementRef::wrap)
        .filter(|element| {
            element.value().attr("itemscope").is_some()
                && element.value().attr("itemprop").is_none()
        })
        .collect();
    elements
        .into_iter()
        .map(|element| document.item(element, &mut Vec::new()))
        .collect()
}

/// How many items deep a value is read: an item deeper is written
/// `"ERROR"` as a loop is, so a hostile document cannot exhaust the stack.
const MAX_DEPTH: usize = 128;

/// What reading the items needs of the whole document.
struct Document<'a> {
    base: Option<Url>,
    /// Each element's place in tree order.
    order: HashMap<NodeId, usize>,
    /// The first element of each ID.
    ids: HashMap<&'a str, ElementRef<'a>>,
}

impl<'a> Document<'a> {
    fn of(tree: &'a Html, base_url: &str) -> Self {
        let mut order = HashMap::new();
        let mut ids = HashMap::new();
        for (place, node) in tree.tree.root().descendants().enumerate() {
            order.insert(node.id(), place);
            if let Some(element) = ElementRef::wrap(node)
                && let Some(id) = element.value().attr("id")
            {
                ids.entry(id).or_insert(element);
            }
        }
        Self {
            base: Url::parse(base_url).ok(),
            order,
            ids,
        }
    }

    /// The item `element` is; `memory` holds the items being read around
    /// it, outermost first.
    fn item(&self, element: ElementRef<'a>, memory: &mut Vec<NodeId>) -> MicrodataItem {
        memory.push(element.id());
        let attribute = |name| element.value().attr(name);
        let mut properties: BTreeMap<String, Vec<MicrodataValue>> = BTreeMap::new();
        for property in self.properties(element) {
            let value = if property.value().attr("itemscope").is_some() {
                if memory.len() >= MAX_DEPTH || memory.contains(&property.id()) {
                    MicrodataValue::Text("ERROR".to_owned())
                } else {
                    MicrodataValue::Item(self.item(property, memory))
                }
            } else {
                MicrodataValue::Text(self.value(property))
            };
            for name in tokens(property.value().attr("itemprop")) {
                properties.entry(name).or_default().push(value.clone());
            }
        }
        memory.pop();
        MicrodataItem {
            types: tokens(attribute("itemtype")),
            id: attribute("itemid").and_then(|id| self.resolve(id)),
            properties,
        }
    }

    /// The properties of the item `root` is, in tree order: the HTML
    /// Standard's crawl over its children and the elements its `itemref`
    /// names, which stops at each item it meets.
    fn properties(&self, root: ElementRef<'a>) -> Vec<ElementRef<'a>> {
        let mut pending: Vec<ElementRef<'a>> = root.child_elements().collect();
        pending.extend(
            tokens(root.value().attr("itemref"))
                .iter()
                .filter_map(|id| self.ids.get(id.as_str()).copied()),
        );
        let mut seen = HashSet::from([root.id()]);
        let mut results = Vec::new();
        while let Some(current) = pending.pop() {
            if !seen.insert(current.id()) {
                continue;
            }
            if current.value().attr("itemscope").is_none() {
                pending.extend(current.child_elements());
            }
            if !tokens(current.value().attr("itemprop")).is_empty() {
                results.push(current);
            }
        }
        results.sort_by_key(|element| self.order.get(&element.id()).copied());
        results
    }

    /// The value of the property `element` is, as the HTML Standard reads
    /// each element.
    fn value(&self, element: ElementRef<'a>) -> String {
        let attribute = |name| element.value().attr(name);
        let url = |name| {
            attribute(name)
                .and_then(|value| self.resolve(value))
                .unwrap_or_default()
        };
        match element.value().name() {
            "meta" => attribute("content").unwrap_or_default().to_owned(),
            "audio" | "embed" | "iframe" | "img" | "source" | "track" | "video" => url("src"),
            "a" | "area" | "link" => url("href"),
            "object" => url("data"),
            "data" | "meter" => attribute("value").unwrap_or_default().to_owned(),
            "time" => {
                attribute("datetime").map_or_else(|| element.text().collect(), ToOwned::to_owned)
            }
            _ => element.text().collect(),
        }
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

/// The unique tokens of an attribute's value, in order; none when it is
/// absent.
fn tokens(value: Option<&str>) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for token in value.unwrap_or_default().split_ascii_whitespace() {
        if !tokens.iter().any(|seen| seen == token) {
            tokens.push(token.to_owned());
        }
    }
    tokens
}
