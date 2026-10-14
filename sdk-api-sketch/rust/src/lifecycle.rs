//! Entry points and lifecycle (ADR 0078, decided in #465).
//!
//! Logs are `tracing` events under `goodfetch::…` targets; the crate never installs a subscriber.
//! The Rust crate links the engine (`EngineChoice::Linked`) and borrows a browser through
//! `session.source = cdp_url`, never through a relay. The Run's start verbs, Sessions and the
//! page API are placeholders owned by #478, #476 and #472; the error type is #466's.

/// Which engine [`Goodfetch::connect`] reaches.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum EngineChoice {
    /// ADR 0024's rule: a Daemon when one is configured or running, otherwise the linked engine.
    #[default]
    Auto,
    /// The engine linked into this program.
    Linked,
    /// The user's Daemon on this machine.
    Daemon,
    /// An Exposed Daemon or Goodfetch Cloud, by address and token.
    Address {
        /// The endpoint.
        url: String,
        /// The token, which #476 turns into a Secret reference.
        token: String,
    },
}

/// Options of [`Goodfetch::connect`]. Where the other settings go is #470's.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ConnectOptions {
    /// Which engine to reach.
    pub engine: EngineChoice,
}

/// The kind of engine a connection reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineKind {
    /// The engine linked into this program.
    Linked,
    /// A Daemon on this machine.
    Daemon,
    /// An Exposed Daemon or Goodfetch Cloud.
    Remote,
}

/// The engine a connection reached, reported for diagnosis (G6).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EngineInfo {
    /// The kind reached.
    pub kind: EngineKind,
    /// The engine's version.
    pub version: String,
    /// The protocol's `major.minor`.
    pub protocol_version: String,
}

/// Placeholder error carrying an Error code such as `closed`; its shape is #466's.
#[derive(Debug)]
pub struct Error {
    code: String,
}

impl Error {
    /// The Error code.
    pub fn code(&self) -> &str {
        &self.code
    }
}

/// The engine connection: the entry point. Each one is independent, and many may be open in one process.
///
/// Dropping it without [`Goodfetch::close`] fires the cancellation tree without waiting and emits a
/// `tracing` warning; Runs resume from their last Checkpoint.
#[derive(Debug)]
pub struct Goodfetch {
    _private: (),
}

impl Goodfetch {
    /// Starts or finds the engine and completes `hello` before returning.
    pub async fn connect(options: ConnectOptions) -> Result<Self, Error> {
        let _ = options;
        unimplemented!()
    }

    /// Opens a connection, lends it to `body`, and closes it on every return and error.
    pub async fn scope<T>(
        options: ConnectOptions,
        body: impl AsyncFnOnce(&Goodfetch) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let _ = (options, body);
        unimplemented!()
    }

    /// The engine this connection reached.
    pub fn engine(&self) -> &EngineInfo {
        unimplemented!()
    }

    /// Collie, reached without I/O.
    pub fn collie(&self) -> Collie<'_> {
        unimplemented!()
    }

    /// Cancels in-flight calls, closes every Run it opened, then stops a linked engine or
    /// disconnects from a Daemon. Idempotent; takes `&self` so it works through an `Arc`.
    pub async fn close(&self) -> Result<(), Error> {
        unimplemented!()
    }
}

impl Drop for Goodfetch {
    /// Never blocks and never fails (C-DTOR-FAIL, C-DTOR-BLOCK).
    fn drop(&mut self) {}
}

/// Collie, borrowed from its connection.
#[derive(Debug)]
pub struct Collie<'a> {
    _connection: &'a Goodfetch,
}

impl Collie<'_> {
    /// Placeholder start verb: its spelling and options are #478's.
    pub async fn start(&self) -> Result<Run, Error> {
        unimplemented!()
    }
}

/// A Run, always opened explicitly. Closing an unfinished Run stops it with a Checkpoint,
/// resumable by its ID.
#[derive(Debug)]
pub struct Run {
    _private: (),
}

impl Run {
    /// The Run's ID.
    pub fn id(&self) -> &str {
        unimplemented!()
    }

    /// Opens a page. Its Session is #476's.
    pub async fn new_page(&self) -> Result<Page, Error> {
        unimplemented!()
    }

    /// Stops the Run if unfinished, after closing its children. Idempotent.
    pub async fn close(&self) -> Result<(), Error> {
        unimplemented!()
    }
}

/// A page Goodfetch drives. Its API is #472's. Closing it closes only a tab Goodfetch opened.
#[derive(Debug)]
pub struct Page {
    _private: (),
}

impl Page {
    /// Closes the tab. Idempotent.
    pub async fn close(&self) -> Result<(), Error> {
        unimplemented!()
    }
}
