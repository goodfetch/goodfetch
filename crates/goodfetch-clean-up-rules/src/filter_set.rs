//! The one recipe every Filter list is compiled with (ADR 0047): its text
//! and Brave's unbreak list in one `adblock` filter set, each parsed with
//! the default options, so the embedded lists the build compiles and a
//! downloaded version the engine compiles block alike; and each list's
//! EasyList Cookie sections split out into a removal-only filter set of
//! cosmetic rules alone (no network rules, no scriptlets), so they never
//! block a request or hide on the live page (issue 2379). The build script
//! includes this file.

use adblock::filters::cosmetic::CosmeticFilterMask;
use adblock::lists::{FilterSet, ParseOptions, ParsedLine, RuleTypes, parse_filter};

/// A Filter list split into its normal rules (`live`) and its EasyList
/// Cookie sections (`cookies`), which only remove elements from extracted
/// content (ADR 0047, issue 2379).
pub(crate) struct SplitSections {
    /// Every section outside `easylist_cookie`.
    pub(crate) live: String,
    /// Every section whose marker names `easylist_cookie`.
    pub(crate) cookies: String,
}

/// Splits `text` into its normal sections and its EasyList Cookie sections:
/// a cookie section begins at its marker,
/// `! *** easylist:easylist_cookie/… ***` in easylist.to's builds or
/// `! easylist_cookie_….txt` in fanboy.co.nz's, and runs to the next marker
/// of either form.
pub(crate) fn split_cookie_sections(text: &str) -> SplitSections {
    let mut live = String::with_capacity(text.len());
    let mut cookies = String::new();
    let mut in_cookie = false;
    for line in text.split_inclusive('\n') {
        if let Some(section) = section_marker(line.trim_end()) {
            in_cookie = section.contains("easylist_cookie");
        }
        if in_cookie {
            cookies.push_str(line);
        } else {
            live.push_str(line);
        }
    }
    SplitSections { live, cookies }
}

/// The section a marker line names: `easylist:<path>` in `! *** … ***`, or
/// the `<name>.txt` of `! <name>.txt`; none for any other line.
fn section_marker(line: &str) -> Option<&str> {
    let comment = line.strip_prefix('!')?.trim();
    if let Some(inner) = comment
        .strip_prefix("***")
        .and_then(|rest| rest.strip_suffix("***"))
    {
        return Some(inner.trim());
    }
    let name = comment.strip_suffix(".txt")?;
    (!name.is_empty() && !name.contains(char::is_whitespace)).then_some(comment)
}

/// The filter set of the Filter list `list`, with `unbreak`, Brave's
/// unbreak list, whose exceptions then apply whichever lists are on.
pub(crate) fn of(list: String, unbreak: String) -> FilterSet {
    let mut set = FilterSet::new(false);
    set.add_filter_list(list, ParseOptions::default());
    set.add_filter_list(unbreak, ParseOptions::default());
    set
}

/// The removal-only filter set of a list's cookie sections (`cookies`),
/// with the cosmetic exceptions (`#@#`) of `unbreak`: keeps only
/// non-scriptlet cosmetic rules so no network rule blocks and no scriptlet
/// runs; returns `None` when `cookies` holds no cosmetic rule.
pub(crate) fn removal_only(cookies: &str, unbreak: &str) -> Option<FilterSet> {
    let cosmetic = ParseOptions {
        rule_types: RuleTypes::CosmeticOnly,
        ..ParseOptions::default()
    };
    let mut kept = String::new();
    for line in cookies.lines() {
        if let Ok(ParsedLine::Cosmetic(filter)) = parse_filter(line, false, cosmetic)
            && !filter.mask.contains(CosmeticFilterMask::SCRIPT_INJECT)
        {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if kept.is_empty() {
        return None;
    }
    let mut unbreak_exceptions = String::new();
    for line in unbreak.lines() {
        if let Ok(ParsedLine::Cosmetic(filter)) = parse_filter(line, false, cosmetic)
            && filter.mask.contains(CosmeticFilterMask::UNHIDE)
            && !filter.mask.contains(CosmeticFilterMask::SCRIPT_INJECT)
        {
            unbreak_exceptions.push_str(line);
            unbreak_exceptions.push('\n');
        }
    }
    let mut set = FilterSet::new(false);
    set.add_filter_list(kept, cosmetic);
    if !unbreak_exceptions.is_empty() {
        set.add_filter_list(unbreak_exceptions, cosmetic);
    }
    Some(set)
}

/// The lines of the Filter list `list` that `adblock` reads as a
/// `$generichide` (or `$ghide`) exception, a `$badfilter` of one included,
/// each ending in a newline: compiled with the unbreak list's own, they
/// make an engine whose `check_generic_hide` answers as the list's whole
/// engine does, for every URL, since `adblock` files each such filter apart
/// by its own options and a `$badfilter` cancels only a filter of those
/// options.
pub(crate) fn generic_hide_lines(list: &str) -> String {
    let mut kept = String::new();
    for line in list.lines() {
        let lower = line.to_ascii_lowercase();
        if !(lower.contains("generichide") || lower.contains("ghide")) {
            continue;
        }
        if let Ok(ParsedLine::Network(filter)) = parse_filter(line, false, ParseOptions::default())
            && filter.is_generic_hide()
        {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept
}
