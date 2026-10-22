//! Every transition table matches its state, then its event, with no
//! wildcard (ADR 0128). `#![deny(clippy::wildcard_enum_match_arm)]` refuses
//! a wildcard in a `match` on an enum, but not one inside a tuple pattern,
//! nor an `if let … else`, a `let … else` or a `matches!`, which would each
//! let a new state or event through unseen: this test reads each table's
//! source and refuses those too, and its own cases show it finds each.

use rstest::rstest;
use syn::visit::{self, Visit};

/// Each module that holds a transition table, by its path in the crate.
const TABLES: [(&str, &str); 2] = [
    ("runs::lifecycle", include_str!("../runs/lifecycle.rs")),
    ("transition::engine", include_str!("engine.rs")),
];

/// What a table's module does that ADR 0128 refuses, outside its tests.
fn refused_matches(source: &str) -> Vec<&'static str> {
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

/// Covers: story:3-151, story:3-152
#[rstest]
#[case::the_runs_table(TABLES[0])]
#[case::the_engines_table(TABLES[1])]
fn a_transition_table_matches_its_state_and_event_with_no_wildcard(
    #[case] (module, source): (&str, &str),
) {
    assert_eq!(refused_matches(source), Vec::<&str>::new(), "{module}");
}

const DENIED: &str = "#![deny(clippy::wildcard_enum_match_arm)]\n";

/// Covers: story:3-151, story:3-152
#[rstest]
#[case::a_wildcard_arm(
    "fn apply(state: S) -> u8 { match state { S::A => 1, _ => 2 } }",
    vec!["a `_` arm"]
)]
#[case::a_wildcard_inside_an_or_pattern(
    "fn apply(state: S) -> u8 { match state { S::A => 1, S::B | _ => 2 } }",
    vec!["a `_` arm"]
)]
#[case::a_binding_that_catches_every_value(
    "fn apply(state: S) -> S { match state { S::A => S::B, other => other } }",
    vec!["an arm that binds every value"]
)]
#[case::a_match_on_a_tuple(
    "fn apply(state: S, event: E) -> u8 { match (state, event) { (S::A, E::X) => 1, (S::B, E::X) => 2 } }",
    vec!["a `match` on a tuple"]
)]
#[case::an_if_let(
    "fn apply(state: S) -> u8 { if let S::A = state { 1 } else { 2 } }",
    vec!["an `if let` or a `let` chain"]
)]
#[case::a_let_else(
    "fn apply(state: S) -> u8 { let S::A = state else { return 2 }; 1 }",
    vec!["a `let … else`"]
)]
#[case::a_matches_macro(
    "fn apply(state: S) -> bool { matches!(state, S::A) }",
    vec!["a `matches!`"]
)]
fn the_check_finds_each_match_that_could_let_a_new_case_through(
    #[case] table: &str,
    #[case] expected: Vec<&str>,
) {
    assert_eq!(refused_matches(&format!("{DENIED}{table}")), expected);
}

/// Covers: story:3-151, story:3-152
#[test]
fn the_check_finds_a_module_that_does_not_deny_the_lint() {
    let source = "#![deny(clippy::pedantic)]\nfn apply(state: S) -> u8 { match state { S::A => 1, S::B => 2 } }";

    assert_eq!(
        refused_matches(source),
        ["the module does not deny clippy::wildcard_enum_match_arm"]
    );
}

/// The table's tests may match as they like.
///
/// Covers: story:3-151, story:3-152
#[test]
fn the_check_leaves_out_the_tables_tests() {
    let source = format!(
        "{DENIED}fn apply(state: S) -> u8 {{ match state {{ S::A => 1, S::B => 2 }} }}\n#[cfg(test)]\nmod tests {{ fn any(state: S) -> bool {{ matches!(state, _) }} }}"
    );

    assert_eq!(refused_matches(&source), Vec::<&str>::new());
}
