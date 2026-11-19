//! Why a Connection or one of its calls failed. The error group classes and
//! the classes of Collie's Error codes replace [`Error::Failed`]'s one shape
//! with the ticket that adds them (spec #606).

use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::jsonrpc::ErrorObject;

/// Why a Connection or one of its calls failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The transport could not reach the engine.
    #[error("could not reach the engine: {0}")]
    Unreachable(#[source] std::io::Error),
    /// The Connection is closed, by `close()` or by its engine, so the call
    /// was never answered.
    #[error("the Connection is closed")]
    Closed,
    /// Another stream already serves the engine's requests of `method`.
    #[error("the engine's requests of `{method}` are already served")]
    AlreadyServed {
        /// The method.
        method: String,
    },
    /// The call's timeout ran out before its reply came.
    #[error("the call timed out")]
    TimedOut,
    /// The engine answered the call with a failure.
    #[error("{}: {}", .0.data.code, .0.message)]
    Failed(Box<ErrorObject>),
    /// A call's params or result did not fit its type.
    #[error("a message did not fit its type: {0}")]
    Shape(#[source] serde_json::Error),
    /// The engine the SDK runs in the program's own process, its linked
    /// engine, could not start or failed (ADR 0087).
    #[error("the linked engine failed: {0}")]
    Engine(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The file the engine keeps a response's body in could not be read
    /// whole, or no longer holds the body's bytes. Its source is the OS's
    /// error.
    #[error("could not read the response's body from `{}`: {source}", .path.display())]
    BodyUnreadable {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: io::Error,
    },
    /// A response's body is not JSON, or not the JSON the program reads
    /// it as: `response_not_json` (ADR 0085).
    #[error("response_not_json: the body of {url} is not the JSON asked for: {source}")]
    ResponseNotJson {
        /// The response's URL.
        url: String,
        /// Why.
        #[source]
        source: serde_json::Error,
    },
    /// The program's path cannot be written for a reason the path causes,
    /// such as a folder the user cannot write: `path_not_writable`
    /// (ADR 0116). Its source is the OS's error, where there is one.
    #[error(transparent)]
    PathNotWritable(Box<FileFailure>),
    /// The program's storage failed while a file was written to its path,
    /// such as when it is full: `file_write_failed`, side `program`
    /// (ADR 0116). Its source is the OS's error.
    #[error(transparent)]
    FileWriteFailed(Box<FileFailure>),
}

/// Why a file did not reach the program's path, which holds what it held
/// before: the details of [`Error::PathNotWritable`] and
/// [`Error::FileWriteFailed`].
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct FileFailure {
    pub(crate) reason: &'static str,
    pub(crate) os_code: Option<&'static str>,
    pub(crate) side: Option<&'static str>,
    pub(crate) path: PathBuf,
    pub(crate) message: String,
    pub(crate) fix: String,
    #[source]
    pub(crate) cause: Option<io::Error>,
}

impl FileFailure {
    /// ADR 0116's reason, such as `permission_denied` or `storage_full`.
    pub fn reason(&self) -> &'static str {
        self.reason
    }

    /// The OS's name of the error, such as `EACCES`, where it gives one.
    pub fn os_code(&self) -> Option<&'static str> {
        self.os_code
    }

    /// Whose storage failed, `program`, for a `file_write_failed`; none
    /// for a `path_not_writable`.
    pub fn side(&self) -> Option<&'static str> {
        self.side
    }

    /// The path that was to be written, as the program gave it.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What to do about it, naming the call that wrote.
    pub fn fix(&self) -> &str {
        &self.fix
    }
}
