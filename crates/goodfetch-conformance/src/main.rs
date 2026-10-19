//! `goodfetch-conformance`'s command line. `read <file>` loads one result
//! document through `goodfetch-protocol`'s reader, which recomputes every
//! outcome it stores (ADR 0063), so a runner that is not written in Rust,
//! such as the TypeScript and Python helpers' tests, checks that the reader
//! accepts what it wrote. It exits 0 when the reader accepts the document,
//! and 1, with the reason on standard error, otherwise.

use std::fmt;
use std::io;
use std::path::PathBuf;

use goodfetch_protocol::result_document::{ReadError, read};

fn main() -> Result<(), Failure> {
    let mut arguments = std::env::args_os().skip(1);
    let (Some(command), Some(file), None) = (arguments.next(), arguments.next(), arguments.next())
    else {
        return Err(Failure::Usage);
    };
    if command != "read" {
        return Err(Failure::Usage);
    }
    let path = PathBuf::from(file);
    #[expect(
        clippy::disallowed_methods,
        reason = "the command reads one file and runs no async worker"
    )]
    let bytes = std::fs::read(&path).map_err(|source| Failure::Unreadable {
        path: path.clone(),
        source,
    })?;
    read(&bytes)
        .map(drop)
        .map_err(|source| Failure::Refused { path, source })
}

/// Why `read` exits 1.
#[derive(thiserror::Error)]
enum Failure {
    #[error("usage: goodfetch-conformance read <file>")]
    Usage,
    #[error("{} cannot be read: {source}", path.display())]
    Unreadable { path: PathBuf, source: io::Error },
    #[error("the reader refuses {}: {source}", path.display())]
    Refused { path: PathBuf, source: ReadError },
}

/// `main` prints an error it returns with `Debug`: the message, not the
/// variant's fields.
impl fmt::Debug for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
