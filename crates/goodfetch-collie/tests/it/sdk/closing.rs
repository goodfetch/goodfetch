//! Closing the tree a Rust program holds (ADR 0078, issue 2928), the
//! lifecycle client conformance cases in Rust's `linked` mode: each value
//! closes its children with it, a closed value refuses every later call
//! with [`Error::Closed`] before sending it, a second close does nothing,
//! a running Run's close stops it resumable, and the Connection's close
//! stops its linked engine. The Scripted engine's Daemon plays the same
//! cases, and a failed close, in `goodfetch-collie-client`'s tests.

use goodfetch_collie::{Collie, Error, RunId, StartedAs};
use goodfetch_test_support::{TestEnv, browser_cache_in_home, real_time_runtime};

use super::options;

/// A URL the closed values' calls would load, which none sends.
const SHOP: &str = "https://shop.test/";

/// The `children-close-with-their-parent` case: a Session's close closes
/// its page, whose own close then does nothing, and the Connection's
/// closes its Run, its Sessions and their pages, which refuse every later
/// call with [`Error::Closed`].
/// Covers: story:11-16
#[test]
fn children_close_with_their_parent_and_refuse_every_later_call() {
    let env = TestEnv::new();
    browser_cache_in_home(&env).unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let session = run.new_session().await.unwrap();
        let session_page = session.new_page().await.unwrap();

        session.close().await.unwrap();
        let page_closed = session_page.close().await;
        let after_session = session_page.goto(SHOP).await;
        let run_page = run.new_page().await.unwrap();
        collie.close().await.unwrap();
        let closed_again = collie.close().await;
        let after_connection = (
            run_page.goto(SHOP).await,
            session.new_page().await,
            run.finish().await,
        );

        assert!(page_closed.is_ok(), "{page_closed:?}");
        assert!(closed_again.is_ok(), "{closed_again:?}");
        assert!(
            matches!(after_session, Err(Error::Closed)),
            "{after_session:?}"
        );
        assert!(
            matches!(
                after_connection,
                (Err(Error::Closed), Err(Error::Closed), Err(Error::Closed))
            ),
            "{after_connection:?}"
        );
    });
}

/// The `closed-after-close` case: a Run and a Connection each closed
/// twice, after which the Run's finish and a start fail with
/// [`Error::Closed`].
/// Covers: story:11-17
#[test]
fn a_closed_run_and_connection_close_again_and_refuse_every_later_call() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();

        run.close().await.unwrap();
        let run_closed_again = run.close().await;
        collie.close().await.unwrap();
        let closed_again = collie.close().await;
        let finished = run.finish().await;
        let started = collie.start().await;

        assert!(run_closed_again.is_ok(), "{run_closed_again:?}");
        assert!(closed_again.is_ok(), "{closed_again:?}");
        assert!(matches!(finished, Err(Error::Closed)), "{finished:?}");
        assert!(matches!(started, Err(Error::Closed)), "{started:?}");
    });
}

/// The `close-stops-a-running-run` case: closing a running Run stops it,
/// so a start of its Run ID resumes it, and closing the Run once it
/// finished succeeds.
/// Covers: story:11-18
#[test]
fn a_closed_running_run_resumes_and_a_finished_one_closes() {
    let env = TestEnv::new();
    let nightly = RunId::parse("nightly").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let first = collie.start().run_id(&nightly).await.unwrap();

        first.close().await.unwrap();
        let resumed = collie.start().run_id(&nightly).await.unwrap();
        resumed.finish().await.unwrap();
        let closed = resumed.close().await;
        collie.close().await.unwrap();

        assert_eq!(resumed.started_as(), StartedAs::Resumed);
        assert!(closed.is_ok(), "{closed:?}");
    });
}

/// The `connection-close-stops-its-engine` case: the Connection's close
/// stops its running Run and its linked engine, after which the Run
/// refuses its finish with [`Error::Closed`], and a new Connection on the
/// same Goodfetch home resumes the Run.
/// Covers: story:11-19
#[test]
fn a_connection_close_stops_its_run_and_its_linked_engine() {
    let env = TestEnv::new();
    let nightly = RunId::parse("nightly").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().run_id(&nightly).await.unwrap();

        collie.close().await.unwrap();
        let finished = run.finish().await;
        let again = Collie::connect(options(&env)).await.unwrap();
        let resumed = again.start().run_id(&nightly).await.unwrap();
        resumed.finish().await.unwrap();
        again.close().await.unwrap();

        assert!(matches!(finished, Err(Error::Closed)), "{finished:?}");
        assert_eq!(resumed.started_as(), StartedAs::Resumed);
    });
}
