//! Live hiding's style sheet (ADR 0047): the text of the sheet that hides,
//! on a live page, what the cosmetic rules of the lists that are on match.
//! On Chromium it goes into a sheet `CSS.createStyleSheet` makes, which no
//! element of the document owns, so the site cannot see it; procedural
//! rules join it only where they are plain CSS ([`PageRules::css_rules`]).
//!
//! [`PageRules::css_rules`]: crate::cosmetic::PageRules::css_rules

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::cosmetic::CssRule;

/// The sheet that applies `rules` but those whose selectors the guard
/// lifted, `lifted`: one CSS rule each, so that a selector a browser does
/// not parse drops only its own.
pub fn sheet(rules: &[CssRule], lifted: &BTreeSet<String>) -> String {
    let mut text = String::new();
    for rule in rules {
        if lifted.contains(&rule.selector) {
            continue;
        }
        writeln!(text, "{} {{ {}; }}", rule.selector, rule.style)
            .expect("invariant: writing to a String cannot fail");
    }
    text
}
