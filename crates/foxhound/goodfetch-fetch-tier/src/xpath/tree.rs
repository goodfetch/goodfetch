//! A served document as `XPath`'s data model holds it (§5), built once
//! from `dom_query`'s parse: its nodes in document order, each element's
//! attributes right after it and before its children, so a node's index is
//! its place in document order and its descendants are the indices up to
//! its end. As in a browser's DOM, a `<template>`'s contents are no
//! children of it, and the doctype is a child of the root that only
//! `node()` finds.

use std::collections::HashMap;

use dom_query::{NodeData, NodeId, NodeRef};

/// The namespace of HTML's elements.
pub(super) const HTML: &str = "http://www.w3.org/1999/xhtml";

/// The namespace of `xmlns` attributes, which no axis reaches.
pub(super) const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

/// The namespace of `xml:lang`.
pub(super) const XML: &str = "http://www.w3.org/XML/1998/namespace";

/// A document's nodes in document order; the root is the first.
pub(super) struct Tree {
    nodes: Vec<Node>,
    /// The first element in document order with each `id`.
    ids: HashMap<String, usize>,
}

/// One node.
pub(super) struct Node {
    pub(super) kind: Kind,
    /// Its parent: an attribute's is its element; the root has none.
    pub(super) parent: Option<usize>,
    /// Its children, attributes aside, in document order.
    pub(super) children: Vec<usize>,
    /// Its attributes, in document order.
    pub(super) attributes: Vec<usize>,
    /// The index past its last descendant.
    pub(super) end: usize,
    /// The node of `dom_query`'s tree it is, for an element.
    pub(super) source: Option<NodeId>,
}

/// What a node is.
pub(super) enum Kind {
    Root,
    Element(Name),
    Attribute(Name, String),
    Text(String),
    Comment(String),
    Instruction { target: String, data: String },
    Doctype,
}

/// An element's or an attribute's expanded name, with its prefix.
pub(super) struct Name {
    pub(super) prefix: Option<String>,
    /// Its namespace's URI; empty for none.
    pub(super) namespace: String,
    pub(super) local: String,
}

impl Name {
    fn of(name: &html5ever::QualName) -> Self {
        Self {
            prefix: name.prefix.as_ref().map(ToString::to_string),
            namespace: name.ns.to_string(),
            local: name.local.to_string(),
        }
    }
}

/// A step of the walk that builds a tree: a node of `dom_query`'s to add
/// under a parent, or the end of an element's descendants.
enum Visit {
    Enter(NodeId, usize),
    Leave(usize),
}

impl Tree {
    /// `document`'s tree.
    pub(super) fn of(document: &dom_query::Document) -> Self {
        let root = document.root();
        let mut tree = Self {
            nodes: vec![Node {
                kind: Kind::Root,
                parent: None,
                children: Vec::new(),
                attributes: Vec::new(),
                end: 1,
                source: None,
            }],
            ids: HashMap::new(),
        };
        let mut visits = vec![Visit::Leave(0)];
        visits.extend(children(&root, 0));
        while let Some(visit) = visits.pop() {
            match visit {
                Visit::Leave(index) => tree.nodes[index].end = tree.nodes.len(),
                Visit::Enter(id, parent) => {
                    let Some(node) = document.tree.get(&id) else {
                        continue;
                    };
                    if let Some(index) = tree.add(&node, parent) {
                        visits.push(Visit::Leave(index));
                        visits.extend(children(&node, index));
                    }
                }
            }
        }
        tree
    }

    /// Adds `node` under `parent` with its attributes, answering its index
    /// when its children follow it; a fragment is skipped.
    fn add(&mut self, node: &NodeRef<'_>, parent: usize) -> Option<usize> {
        let index = self.nodes.len();
        let mut attributes = Vec::new();
        let kind = node.query(|inner| match &inner.data {
            NodeData::Document | NodeData::Fragment => None,
            NodeData::Doctype { .. } => Some(Kind::Doctype),
            NodeData::Text { contents } => Some(Kind::Text(contents.to_string())),
            NodeData::Comment { contents } => Some(Kind::Comment(contents.to_string())),
            NodeData::ProcessingInstruction { target, contents } => Some(Kind::Instruction {
                target: target.to_string(),
                data: contents.to_string(),
            }),
            NodeData::Element(element) => {
                attributes = element
                    .attrs
                    .iter()
                    .map(|attribute| (Name::of(&attribute.name), attribute.value.to_string()))
                    .collect();
                Some(Kind::Element(Name::of(&element.name)))
            }
        })??;
        let element = matches!(kind, Kind::Element(_));
        self.nodes.push(Node {
            kind,
            parent: Some(parent),
            children: Vec::new(),
            attributes: Vec::new(),
            end: index + 1,
            source: element.then_some(node.id),
        });
        self.nodes[parent].children.push(index);
        for (name, value) in attributes {
            let attribute = self.nodes.len();
            if name.local == "id" && name.namespace.is_empty() {
                self.ids.entry(value.clone()).or_insert(index);
            }
            self.nodes.push(Node {
                kind: Kind::Attribute(name, value),
                parent: Some(index),
                children: Vec::new(),
                attributes: Vec::new(),
                end: attribute + 1,
                source: None,
            });
            self.nodes[index].attributes.push(attribute);
        }
        element.then_some(index)
    }

    /// How many nodes it holds.
    pub(super) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The node at `index`.
    pub(super) fn node(&self, index: usize) -> &Node {
        &self.nodes[index]
    }

    /// Whether the node at `index` is an attribute.
    pub(super) fn is_attribute(&self, index: usize) -> bool {
        matches!(self.nodes[index].kind, Kind::Attribute(..))
    }

    /// The first element in document order whose `id` is `id`.
    pub(super) fn element_by_id(&self, id: &str) -> Option<usize> {
        self.ids.get(id).copied()
    }

    /// The string-value of the node at `index` (§5): an element's and the
    /// root's, the text of every text node it holds, in document order.
    pub(super) fn string_value(&self, index: usize) -> String {
        let node = &self.nodes[index];
        match &node.kind {
            Kind::Root | Kind::Element(_) => (index + 1..node.end)
                .filter_map(|inner| match &self.nodes[inner].kind {
                    Kind::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect(),
            Kind::Attribute(_, text) | Kind::Text(text) | Kind::Comment(text) => text.clone(),
            Kind::Instruction { data, .. } => data.clone(),
            Kind::Doctype => String::new(),
        }
    }
}

/// The visits that add `node`'s children under `parent`, the first on top.
fn children(node: &NodeRef<'_>, parent: usize) -> Vec<Visit> {
    let mut visits: Vec<Visit> = node
        .children_it(false)
        .map(|child| Visit::Enter(child.id, parent))
        .collect();
    visits.reverse();
    visits
}
