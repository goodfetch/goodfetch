//! `goodfetch-collie` (ADRs 0090, 0095): Collie's Rust SDK. It re-exports the Collie client crate,
//! whose values (Run, Session, Page, errors) a Foxhound handler gets too, and adds the connection,
//! [`Collie`], with Collie's engine linked in (ADR 0024).
//!
//! SDK API sketch: signatures only, no implementation.

pub use goodfetch_collie_client::*;

pub mod blocking;
#[cfg(feature = "test-util")]
pub mod testing;


/// A connection to an engine, Collie's or Foxhound's (ADR 0095): the entry point (ADR 0078). It
/// announces Collie in `hello`, so a Foxhound engine accepts it too.
///
/// A cheap clone of one connection (ADR 0081). Dropping the last clone without [`Collie::close`]
/// fires the cancellation tree without waiting and emits a `tracing` warning; Runs resume from their
/// last Checkpoint.
#[derive(Debug, Clone)]
pub struct Collie {
    _private: (),
}

impl Collie {
    /// Starts or finds the engine and completes `hello` before returning.
    pub async fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// Opens a connection, lends it to `body`, and closes it on every return and error.
    pub async fn scope<T>(options: ConnectOptions, body: impl AsyncFnOnce(&Collie) -> Result<T, Error>) -> Result<T, Error> {
        let _ = (options, body);
        unimplemented!()
    }

    /// The engine this connection reached.
    pub fn engine(&self) -> &EngineInfo {
        unimplemented!()
    }

    /// The install's Saved identities (ADR 0092).
    pub fn identities(&self) -> Identities {
        unimplemented!()
    }

    /// Starts a Run and returns once it is started (ADR 0102). With `.id(..)` naming an existing
    /// Run with the same inputs: resumes it if interrupted ([`run::StartedAs::Resumed`]), returns it
    /// as it is if it finished (`Ended`), or fails with `run_in_use` if it is live, `run_conflict`
    /// for other inputs, `limit_reached` if a used-up Limit stopped it, and `run_state_unavailable`
    /// if its state is gone.
    pub fn start(&self) -> Start {
        unimplemented!()
    }

    /// Opens a Run by its ID to read it, never to drive it: a detached Run, another program's, or
    /// one that ended. Fails with `run_not_found`.
    pub fn open_run(&self, id: &str) -> Call<RunView> {
        let _ = id;
        unimplemented!()
    }

    /// The raw catalog call on the connection, as [`Connection::call`].
    pub fn call(&self, operation: &str) -> RawCall {
        let _ = operation;
        unimplemented!()
    }

    /// The connection's effective settings, with the layer that set each; all of them when `keys`
    /// is empty.
    pub fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        let _ = keys;
        unimplemented!()
    }

    /// The model calls for the declared provider `name`, subscribed when called; a name not
    /// declared with [`ConnectOptions::model_provider`], or a second stream while one is open, fails
    /// with `setting_invalid`. A call waits for its stream until its own deadline (ADR 0101).
    pub fn model_requests(&self, name: &str) -> Result<EventStream<ModelCall>, Error> {
        let _ = name;
        unimplemented!()
    }

    /// The calls for the Proxy provider `name`, which [`ConnectOptions::proxy_provider`] declared
    /// (ADR 0104).
    pub fn proxy_requests(&self, name: &str) -> Result<EventStream<ProxyCall>, Error> {
        let _ = name;
        unimplemented!()
    }

    /// The `caller:` references the engine asks this program to resolve, once
    /// [`ConnectOptions::resolve_secret`] declared it (ADR 0104). Undeclared, or while another
    /// stream is open, it fails with `setting_invalid`. A request waits for its stream until its own
    /// deadline, and one dropped unanswered fails with `secret_unavailable`.
    pub fn secret_requests(&self) -> Result<EventStream<SecretRequest>, Error> {
        unimplemented!()
    }

    /// Cancels in-flight calls, closes every Run it opened, then stops a linked engine or
    /// disconnects from a Daemon, for every clone. Idempotent. Its timeout is the close phase's.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

impl Drop for Collie {
    /// Never blocks and never fails (C-DTOR-FAIL, C-DTOR-BLOCK); acts when the last clone is dropped.
    fn drop(&mut self) {}
}

impl Connection for Collie {
    fn engine(&self) -> &EngineInfo {
        Collie::engine(self)
    }

    fn identities(&self) -> Identities {
        Collie::identities(self)
    }

    fn call(&self, operation: &str) -> RawCall {
        Collie::call(self, operation)
    }

    fn explain_settings(&self, keys: &[SettingKey]) -> Call<Vec<ExplainedSetting>> {
        Collie::explain_settings(self, keys)
    }

    fn close(&self) -> Call<()> {
        Collie::close(self)
    }

    fn start(&self) -> Start {
        Collie::start(self)
    }

    fn open_run(&self, id: &str) -> Call<RunView> {
        Collie::open_run(self, id)
    }
}
