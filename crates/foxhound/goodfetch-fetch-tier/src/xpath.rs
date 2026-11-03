//! `XPath` 1.0 over a served document (ADR 0103's #1002 note, owner
//! decision O23): an expression is read and checked once (`syntax`), the
//! document is held as `XPath`'s data model (`tree`), and the expression is
//! evaluated over it as a browser's `document.evaluate` evaluates it over
//! the same HTML (`evaluate`), so a code-free crawl's `XPath` field reads
//! alike on every tier. An expression a browser refuses, or one that
//! answers no nodes, is refused before any page is read.
//!
//! Written here: no maintained crate evaluates `XPath` 1.0 over an HTML
//! tree with a browser's name matching (the `sxd` crates were last
//! released in 2019, read XML's names only and lack `id()` and `lang()`).

mod evaluate;
mod syntax;
mod tokens;
mod tree;

#[cfg(test)]
mod tests;

use dom_query::NodeId;

use evaluate::{Context, Value};

/// An expression that is no `XPath` 1.0 expression the engine reads to
/// nodes, with why.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{expression}` is no XPath 1.0 expression that finds nodes: {reason}")]
#[non_exhaustive]
pub struct XpathInvalid {
    expression: String,
    reason: String,
}

/// A checked expression that answers nodes.
#[derive(Clone, Debug)]
pub(crate) struct Expression(syntax::Expr);

impl Expression {
    /// Reads `text`.
    ///
    /// # Errors
    ///
    /// When `text` is no `XPath` 1.0 expression, names a namespace prefix
    /// or a variable (neither is ever bound), calls a function the core
    /// library lacks or with a wrong number of arguments, or answers a
    /// value rather than nodes.
    pub(crate) fn parse(text: &str) -> Result<Self, XpathInvalid> {
        syntax::parse(text)
            .map(Self)
            .map_err(|reason| XpathInvalid {
                expression: text.to_owned(),
                reason,
            })
    }
}

/// A served document as `XPath` reads it.
pub(crate) struct Document(tree::Tree);

impl Document {
    /// `document`, as `XPath` reads it.
    pub(crate) fn of(document: &goodfetch_counted_work::ParsedHtml) -> Self {
        Self(tree::Tree::of(document))
    }

    /// The first node in document order that `expression` finds, from the
    /// root, when it is an element: the node of `dom_query`'s tree it is.
    pub(crate) fn first_element(&self, expression: &Expression) -> Option<NodeId> {
        let found = evaluate::evaluate(
            &self.0,
            &expression.0,
            Context {
                node: 0,
                position: 1,
                size: 1,
            },
        );
        match found {
            Value::Nodes(nodes) => self.0.node(*nodes.first()?).source,
            Value::Boolean(_) | Value::Number(_) | Value::String(_) => None,
        }
    }
}
