//! The Clean-up rules manifest's writer for the Compatibility corpus (ADR
//! 0066): a version of the Clean-up rules as the engine keeps the Filter
//! lists it fetched from their authors, at ADR 0059's test `Clock` start,
//! its manifest written by the engine's own writer beside a short list of
//! each kind, into the folder given as the only argument. `xtask compat
//! write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_protocol::clean_up_rules::{
    CleanUpRulesManifest, MANIFEST_FILE, RulesLists, RulesSource,
};
use goodfetch_protocol::formats::CLEAN_UP_RULES;
use goodfetch_protocol::point_in_time;

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

/// When the shortest-lived list expires: Brave's unbreak list's 12 hours.
const EXPIRES: &str = "2026-01-05T21:00:00Z";

/// Each list's file and text.
const LISTS: [(&str, &str); 4] = [
    (
        "easylist.txt",
        "[Adblock Plus 2.0]\n! Title: EasyList\n! Expires: 4 days (update frequency)\n/banner-ad.\n",
    ),
    (
        "easyprivacy.txt",
        "[Adblock Plus 2.0]\n! Title: EasyPrivacy\n! Expires: 4 days (update frequency)\n/beacon.js\n",
    ),
    (
        "fanboy-annoyance.txt",
        "[Adblock Plus 2.0]\n! Title: Fanboy's Annoyance List\n! Expires: 4 days (update frequency)\n/newsletter-popup.\n",
    ),
    (
        "brave-unbreak.txt",
        "[Adblock Plus 2.0]\n! Title: Brave Unbreak\n! Expires: 12 hours\n@@/beacon.js?keep=1\n",
    ),
];

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let manifest = CleanUpRulesManifest {
        format_version: CLEAN_UP_RULES.version,
        source: RulesSource::Upstream,
        published_at: point_in_time::from_rfc3339(START).expect("invariant: an RFC 3339 time"),
        expires_at: Some(
            point_in_time::from_rfc3339(EXPIRES).expect("invariant: an RFC 3339 time"),
        ),
        lists: RulesLists {
            ads: LISTS[0].0.to_owned(),
            trackers: LISTS[1].0.to_owned(),
            annoyances: LISTS[2].0.to_owned(),
            unbreak: LISTS[3].0.to_owned(),
        },
    };
    std::fs::write(
        output.join(MANIFEST_FILE),
        goodfetch_page::manifest_file(&manifest),
    )
    .expect("invariant: the output folder is writable");
    for (file, text) in LISTS {
        std::fs::write(output.join(file), text).expect("invariant: the output folder is writable");
    }
}
