//! The one recipe every Filter list is compiled with (ADR 0047): its text
//! and Brave's unbreak list in one `adblock` filter set, each parsed with
//! the default options, so the embedded lists the build compiles and a
//! downloaded version the engine compiles block alike. The build script
//! includes this file.

use adblock::lists::{FilterSet, ParseOptions, ParsedLine, parse_filter};

/// The filter set of the Filter list `list`, with `unbreak`, Brave's
/// unbreak list, whose exceptions then apply whichever lists are on.
pub(crate) fn of(list: String, unbreak: String) -> FilterSet {
    let mut set = FilterSet::new(false);
    set.add_filter_list(list, ParseOptions::default());
    set.add_filter_list(unbreak, ParseOptions::default());
    set
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
