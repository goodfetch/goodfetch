//! The one recipe every Filter list is compiled with (ADR 0047): its text
//! and Brave's unbreak list in one `adblock` filter set, each parsed with
//! the default options, so the embedded lists the build compiles and a
//! downloaded version the engine compiles block alike. The build script
//! includes this file.

use adblock::lists::{FilterSet, ParseOptions};

/// The filter set of the Filter list `list`, with `unbreak`, Brave's
/// unbreak list, whose exceptions then apply whichever lists are on.
pub(crate) fn of(list: String, unbreak: String) -> FilterSet {
    let mut set = FilterSet::new(false);
    set.add_filter_list(list, ParseOptions::default());
    set.add_filter_list(unbreak, ParseOptions::default());
    set
}
