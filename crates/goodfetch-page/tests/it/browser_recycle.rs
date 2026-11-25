//! A browser's recycle, in the browser pool against the scripted browser
//! (B2's pool cell, ADR 0140): the browser's owner stops and waits for every
//! task it started, its supervision and the readers of its connection among
//! them (ADR 0146; spec #2256, issue 3264).

use goodfetch_engine::metrics::Metrics;

use crate::session_counts::session::alive_around_a_recycle;

#[test]
fn a_recycled_browser_leaves_none_of_its_tasks_running() {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let (before, after) = alive_around_a_recycle(home.path(), Metrics::none());
    assert_eq!(
        after, before,
        "the tasks alive once a browser was recycled and its replacement served a page, against those while the first served one"
    );
}
