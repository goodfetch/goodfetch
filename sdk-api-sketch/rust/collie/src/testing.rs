//! Collie's testing kit (ADRs 0069, 0090, 0105), behind `test-util`: the shared kit of
//! `goodfetch_collie_client::testing`, and a test engine whose connection is [`Collie`]. Crawl
//! handlers and Hooks are Foxhound's, so `run_handler` and `run_hook` are in
//! `goodfetch_foxhound::testing` (ADR 0084).

pub use goodfetch_collie_client::testing::*;

use goodfetch_collie_client::Error;

use crate::Collie;

/// A real engine for one test, linked in-process: a temporary Goodfetch home removed when it
/// closes, a cleared environment, no project file unless named, `offline` (ADR 0043), short
/// durations, and no Model provider unless the test registers or configures one, so a call that
/// needs a model with none fails with `model_not_configured`. Every Run started from
/// [`TestEngine::connection`] that names no Response recording replays an empty one: a request to a
/// site fails with `response_not_recorded`, naming it. Browsers and the Product's executable come
/// from the browser cache of the caller's Goodfetch home, read and never written, or
/// `GOODFETCH_BROWSERS_PATH` and the Product's engine-path variable (ADRs 0024, 0059, 0090, 0105).
#[derive(Debug)]
pub struct TestEngine {
    _private: (),
}

impl TestEngine {
    /// Starts it.
    pub async fn start(options: TestOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// A connection to this engine, [`Collie::connect`](crate::Collie::connect)'s type: everything a program does works
    /// on it.
    pub fn connection(&self) -> &Collie {
        unimplemented!()
    }

    /// Stands for a person's answer about an uncertain Effect of a Run on this engine (ADR 0029).
    /// Only the testing kit has it (ADR 0106).
    pub async fn reconcile(&self, run_id: &str, key: &str, answer: Reconciliation) -> Result<(), Error> {
        let _ = (run_id, key, answer);
        unimplemented!()
    }

    /// Closes the connection and the engine and removes the home. Fails with
    /// `scripted_model_unused` if a Scripted model registered here has replies left. Dropping it
    /// unclosed removes the home too, without that check.
    pub async fn close(self) -> Result<(), Error> {
        unimplemented!()
    }
}

