//! A Session's end, in the browser pool against the scripted browser (B2's
//! cell, ADR 0140): it stops and waits for every task it and its pages
//! started (spec #2256, issue 3262).

use goodfetch_engine::metrics::Metrics;

use crate::session_counts::session::alive_around_a_retirement;

#[test]
fn a_sessions_end_leaves_none_of_its_tasks_or_its_pages_running() {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let (before, after) = alive_around_a_retirement(home.path(), Metrics::none());
    assert_eq!(
        after, before,
        "the tasks alive once the retired Session ended, against those before it opened"
    );
}
