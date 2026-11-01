//! The check that a transition table matches its state, then its event,
//! with no wildcard (ADR 0128). `#![deny(clippy::wildcard_enum_match_arm)]`
//! refuses a wildcard in a `match` on an enum, but not one inside a tuple
//! pattern, nor an `if let … else`, a `let … else` or a `matches!`, which
//! would each let a new state or event through unseen: [`refused_matches`]
//! reads a table's module source and finds those too, and the crate that
//! owns each table runs it on its module.

use syn::visit::{self, Visit};

/// What the table module `source` does that ADR 0128 refuses, outside its
/// tests, each in source order; none for a table that keeps the rule.
///
/// # Panics
///
/// When `source` is not Rust a table's module could be.
pub fn refused_matches(source: &str) -> Vec<&'static str> {
    let file = syn::parse_file(source).expect("invariant: a table's module parses");
    let mut found = Refused::default();
    if !file.attrs.iter().any(denies_wildcards) {
        found
            .0
            .push("the module does not deny clippy::wildcard_enum_match_arm");
    }
    for item in &file.items {
        if !is_test_only(item) {
            found.visit_item(item);
        }
    }
    found.0
}

/// Whether `attribute` is `#![deny(clippy::wildcard_enum_match_arm)]`.
fn denies_wildcards(attribute: &syn::Attribute) -> bool {
    let mut denied = false;
    if matches!(attribute.style, syn::AttrStyle::Inner(_)) && attribute.path().is_ident("deny") {
        let _ = attribute.parse_nested_meta(|lint| {
            let path: Vec<String> = lint
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            denied |= path == ["clippy", "wildcard_enum_match_arm"];
            Ok(())
        });
    }
    denied
}

/// Whether `item` is compiled only for tests.
fn is_test_only(item: &syn::Item) -> bool {
    let attributes = match item {
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        syn::Item::Const(item) => &item.attrs,
        _ => return false,
    };
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_nested_meta(|cfg| {
                    if cfg.path.is_ident("test") {
                        Ok(())
                    } else {
                        Err(cfg.error("not `test`"))
                    }
                })
                .is_ok()
    })
}

/// Each refused construct found, in source order.
#[derive(Default)]
struct Refused(Vec<&'static str>);

impl Refused {
    /// Each `_` arm, and each arm that binds every value, of `pattern`.
    fn arm(&mut self, pattern: &syn::Pat) {
        match pattern {
            syn::Pat::Wild(_) => self.0.push("a `_` arm"),
            syn::Pat::Ident(binding) if binding.subpat.is_none() && is_binding(&binding.ident) => {
                self.0.push("an arm that binds every value");
            }
            syn::Pat::Ident(syn::PatIdent {
                subpat: Some((_, inner)),
                ..
            }) => self.arm(inner),
            syn::Pat::Or(cases) => cases.cases.iter().for_each(|case| self.arm(case)),
            syn::Pat::Paren(inner) => self.arm(&inner.pat),
            syn::Pat::Guard(guarded) => self.arm(&guarded.pat),
            _ => {}
        }
    }
}

/// Whether `ident` names a binding rather than a unit variant or a
/// constant, by Rust's naming conventions.
fn is_binding(ident: &syn::Ident) -> bool {
    ident
        .to_string()
        .starts_with(|first: char| first.is_lowercase() || first == '_')
}

impl<'ast> Visit<'ast> for Refused {
    fn visit_expr_match(&mut self, matching: &'ast syn::ExprMatch) {
        if let syn::Expr::Tuple(_) = &*matching.expr {
            self.0.push("a `match` on a tuple");
        }
        for arm in &matching.arms {
            self.arm(&arm.pat);
        }
        visit::visit_expr_match(self, matching);
    }

    fn visit_expr_let(&mut self, binding: &'ast syn::ExprLet) {
        self.0.push("an `if let` or a `let` chain");
        visit::visit_expr_let(self, binding);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if local
            .init
            .as_ref()
            .is_some_and(|init| init.diverge.is_some())
        {
            self.0.push("a `let … else`");
        }
        visit::visit_local(self, local);
    }

    fn visit_macro(&mut self, called: &'ast syn::Macro) {
        if called.path.is_ident("matches") {
            self.0.push("a `matches!`");
        }
        visit::visit_macro(self, called);
    }
}

#[cfg(test)]
mod tests;
