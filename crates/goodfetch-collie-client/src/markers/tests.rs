//! ADR 0030's run-time warning for a deprecated or Experimental entry
//! (ADR 0099): once per process per name, through `tracing`.

use goodfetch_test_support::logged;

use super::{warn, warn_operation};

/// How many times the captured `log` holds `text`.
fn count(log: &[u8], text: &str) -> usize {
    String::from_utf8_lossy(log).matches(text).count()
}

#[test]
fn a_deprecated_entry_warns_once_per_process_through_tracing() {
    let ((), log) = logged(|| {
        warn("fixture.old", false, Some("Since 0.1: use `fixture.new`."));
        warn("fixture.old", false, Some("Since 0.1: use `fixture.new`."));
    });

    let shown = String::from_utf8_lossy(&log);
    assert_eq!(
        count(
            &log,
            "`fixture.old` is deprecated. Since 0.1: use `fixture.new`."
        ),
        1,
        "{shown}"
    );
    assert!(shown.contains("WARN"), "{shown}");
}

/// Covers: story:11-184
#[test]
fn an_experimental_entry_warns_once_per_process_through_tracing() {
    let ((), log) = logged(|| {
        warn("fixture.trial", true, None);
        warn("fixture.trial", true, None);
    });

    let shown = String::from_utf8_lossy(&log);
    assert_eq!(
        count(
            &log,
            "`fixture.trial` is Experimental: outside semver until it is promoted (ADR 0030)."
        ),
        1,
        "{shown}"
    );
    assert!(shown.contains("WARN"), "{shown}");
}

/// An entry both deprecated and Experimental warns once of each.
#[test]
fn each_marker_of_one_entry_warns_once() {
    let ((), log) = logged(|| {
        warn("fixture.both", true, Some("Since 0.1: going."));
        warn("fixture.both", true, Some("Since 0.1: going."));
    });

    assert_eq!(count(&log, "`fixture.both` is Experimental"), 1);
    assert_eq!(count(&log, "`fixture.both` is deprecated"), 1);
}

/// A hand-written member reads its operation's markers from the catalog
/// and warns of each once, as a generated member does.
#[test]
fn a_hand_written_member_warns_of_its_operations_catalog_markers_once() {
    let ((), log) = logged(|| {
        warn_operation("sample.echo");
        warn_operation("sample.echo");
    });

    let shown = String::from_utf8_lossy(&log);
    assert_eq!(count(&log, "`sample.echo` is Experimental"), 1, "{shown}");
    assert_eq!(
        count(
            &log,
            "`sample.echo` is deprecated. Since 0.1, removed no earlier than 1.0: A sample entry, which the real catalog replaces."
        ),
        1,
        "{shown}"
    );
}
