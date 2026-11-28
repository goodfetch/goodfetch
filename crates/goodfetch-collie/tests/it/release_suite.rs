//! Tests that run in the Release suite alone, on every target: each asks
//! [`skipped_outside_the_release_suite`] first and, elsewhere, says so and
//! skips.

use goodfetch_test_support::{Suite, Variables};

/// Whether the test skips here, outside the Release suite, saying so and
/// `what` it runs on stderr. Asked before the test's
/// [`goodfetch_test_support::TestEnv`], so a skipped test leaves no
/// exercise record.
pub(crate) fn skipped_outside_the_release_suite(what: &str) -> bool {
    let suite = Variables::from_process()
        .suite()
        .expect("GOODFETCH_TEST_SUITE names a suite");
    if suite == Suite::Release {
        return false;
    }
    #[expect(
        clippy::print_stderr,
        reason = "a skipped test says why in its own output, which the test harness keeps"
    )]
    {
        eprintln!("skipped outside the Release suite: {what}");
    }
    true
}
