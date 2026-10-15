//! SDK handlers both Products take (ADR 0104): a program's own Proxy provider, declared on its
//! connection by name, as ADR 0101 declares a Model provider. The Fetch backend and Sink kinds are
//! Foxhound's (ADR 0088), in `goodfetch_foxhound::hooks`.

use crate::results::BlockCause;
use crate::sessions::SecretReference;

/// An SDK handler's own error, given to a call's `fail`. A Goodfetch [`Error`] keeps its code; any
/// other is the kind's `<kind>_handler_failed`, with this error's type and message in its details and
/// its `Display`, so a Rust handler's failure is as diagnosable as a TypeScript one's (G6).
pub type HandlerError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// What a Proxy provider is asked for: one proxy per Session (ADRs 0021, 0092).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProxyRequest {
    /// The Session's ID.
    pub session_id: String,
    /// The registrable domain of the Session's first request.
    pub domain: String,
    /// Set when the Session replaces one retired for this Block cause.
    pub replacing: Option<BlockCause>,
}

/// A proxy, with Playwright's fields as `proxy.*` has them (ADR 0092).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProxyAnswer {
    /// `http://host:port` or `socks5://host:port`.
    pub server: String,
    /// The user name.
    pub username: Option<String>,
    /// A Secret reference, never a value: `caller:` resolves through
    /// the connection's `secret_requests`.
    pub password: Option<SecretReference>,
}

impl ProxyAnswer {
    /// A proxy with no credentials.
    pub fn new(server: &str) -> Self {
        Self { server: server.to_owned(), username: None, password: None }
    }

    /// With credentials.
    pub fn with_credentials(self, username: &str, password: SecretReference) -> Self {
        Self { username: Some(username.to_owned()), password: Some(password), ..self }
    }
}

/// One call to the program's Proxy provider.
#[derive(Debug)]
pub struct ProxyCall {
    _private: (),
}

impl ProxyCall {
    /// What the engine asks.
    pub fn request(&self) -> &ProxyRequest {
        unimplemented!()
    }

    /// Sends the answer to the engine.
    pub fn answer(self, proxy: ProxyAnswer) {
        let _ = proxy;
        unimplemented!()
    }

    /// Fails the call: the Session does not open.
    pub fn fail(self, error: HandlerError) {
        let _ = error;
        unimplemented!()
    }
}

/// Details of a `<kind>_handler_failed`: the handler, then the error as `handler_failed` carries
/// a handler's.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SdkHandlerFailed {
    /// The name it was registered under.
    pub handler: String,
    /// The error's type name.
    pub error_type: String,
    /// Its message.
    pub error_message: String,
    /// The backtrace frames of the program's code, when captured.
    pub stack: Vec<String>,
}

impl crate::lifecycle::ConnectOptions {
    /// Declares the program's Proxy provider `name`, whose calls the connection's `proxy_requests`
    /// reads (ADR 0104).
    pub fn proxy_provider(self, name: &str) -> Self {
        let _ = name;
        unimplemented!()
    }

    /// Declares that the program answers `caller:` Secret references, which the connection's
    /// `secret_requests` reads (ADR 0092). Without it, such a reference fails with
    /// `secret_unavailable`, its `fix` naming this method, whether or not a stream is open.
    pub fn resolve_secret(self) -> Self {
        unimplemented!()
    }
}

crate::results::open_value_type! {
    /// A Hook stage (ADRs 0033, 0104), an open value type (ADR 0099): the stages Collie's Script
    /// Hooks run at are associated constants; Foxhound's own (before a request, after a response,
    /// before a Record's commit, a request's end) are free constants of
    /// `goodfetch_foxhound::hook_stage`.
    HookStage
}

impl HookStage {
    /// Before the browser navigates.
    pub const BEFORE_NAVIGATION: HookStage = HookStage::new("before_navigation");
    /// After the browser navigated, before the engine's own page work.
    pub const AFTER_NAVIGATION: HookStage = HookStage::new("after_navigation");
    /// When the page is ready: in a crawl, just before the handler, on every tier.
    pub const PAGE_READY: HookStage = HookStage::new("page_ready");
}
