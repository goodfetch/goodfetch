//! `goodfetch-conformance`'s command line, through which a runner that is
//! not written in Rust, such as the TypeScript and Python helpers' tests,
//! reaches the reader and the writer:
//! - `read <file>` loads one result document through `goodfetch-protocol`'s
//!   reader, which recomputes every outcome it stores (ADR 0063), and
//!   `read <file> --journey <Journey file>` also compares a `journey`
//!   document with its Journey, as every reader of stored documents does;
//! - `write <case list> <header> <case log> <document>` turns a case log and
//!   the declared case list into a `conformance` document through the writer
//!   (ADR 0069), the header given as JSON, and writes it to `<document>`.
//!
//! It exits 0 when the reader accepts the document or the writer wrote it,
//! and 1, with the reason on standard error, otherwise.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_conformance::{CaseList, CaseLogError, WriteError, read_case_log, write_document};
use goodfetch_protocol::journey::Journey;
use goodfetch_protocol::result_document::{
    ConformanceHeader, LoadedDocument, ReadError, ResultDocument, read, read_journey_document,
};

fn main() -> Result<(), Failure> {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [command, file] if command == "read" => {
            let path = PathBuf::from(file);
            read(&contents(&path)?)
                .map(drop)
                .map_err(|source| Failure::Refused { path, source })
        }
        [command, file, flag, journey] if command == "read" && flag == "--journey" => {
            read_compared(Path::new(file), Path::new(journey))
        }
        [command, case_list, header, log, document] if command == "write" => write(
            Path::new(case_list),
            Path::new(header),
            Path::new(log),
            Path::new(document),
        ),
        _ => Err(Failure::Usage),
    }
}

/// Loads a `journey` document and compares it with the Journey `journey`
/// holds.
fn read_compared(file: &Path, journey: &Path) -> Result<(), Failure> {
    let journey: Journey = toml::from_str(&text(journey)?).map_err(|source| Failure::Journey {
        path: journey.to_path_buf(),
        source,
    })?;
    let refused = |source| Failure::Refused {
        path: file.to_path_buf(),
        source,
    };
    match read(&contents(file)?).map_err(refused)? {
        LoadedDocument::Journey(loaded) => read_journey_document(loaded, &journey)
            .map(drop)
            .map_err(refused),
        LoadedDocument::Conformance(_) => Err(Failure::NotAJourneyDocument {
            path: file.to_path_buf(),
        }),
    }
}

fn write(case_list: &Path, header: &Path, log: &Path, document: &Path) -> Result<(), Failure> {
    let case_list: CaseList =
        toml::from_str(&text(case_list)?).map_err(|source| Failure::CaseList {
            path: case_list.to_path_buf(),
            source,
        })?;
    let header: ConformanceHeader =
        serde_json::from_slice(&contents(header)?).map_err(|source| Failure::Header {
            path: header.to_path_buf(),
            source,
        })?;
    let log = read_case_log(&text(log)?).map_err(|source| Failure::CaseLog {
        path: log.to_path_buf(),
        source,
    })?;
    let written = ResultDocument::Conformance(write_document(header, &case_list, &log)?);
    let bytes = serde_json::to_vec_pretty(&written)
        .expect("invariant: a result document serializes to JSON");
    #[expect(
        clippy::disallowed_methods,
        reason = "the command writes one file and runs no async worker"
    )]
    std::fs::write(document, bytes).map_err(|source| Failure::Unwritable {
        path: document.to_path_buf(),
        source,
    })
}

fn contents(path: &Path) -> Result<Vec<u8>, Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the command reads its files and runs no async worker"
    )]
    std::fs::read(path).map_err(|source| Failure::Unreadable {
        path: path.to_path_buf(),
        source,
    })
}

fn text(path: &Path) -> Result<String, Failure> {
    String::from_utf8(contents(path)?).map_err(|_| Failure::Unreadable {
        path: path.to_path_buf(),
        source: io::Error::from(io::ErrorKind::InvalidData),
    })
}

/// Why the command exits 1.
#[derive(thiserror::Error)]
enum Failure {
    #[error(
        "usage: goodfetch-conformance read <file> [--journey <Journey file>]\n       goodfetch-conformance write <case list> <header> <case log> <document>"
    )]
    Usage,
    #[error("{} cannot be read: {source}", path.display())]
    Unreadable { path: PathBuf, source: io::Error },
    #[error("the reader refuses {}: {source}", path.display())]
    Refused { path: PathBuf, source: ReadError },
    #[error("{} is not a Journey file: {source}", path.display())]
    Journey {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{} is not a `journey` document, so no Journey compares with it", path.display())]
    NotAJourneyDocument { path: PathBuf },
    #[error("{} is not a case list: {source}", path.display())]
    CaseList {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{} is not a conformance document's header: {source}", path.display())]
    Header {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{}: {source}", path.display())]
    CaseLog { path: PathBuf, source: CaseLogError },
    #[error("the writer refuses the case log: {0}")]
    Write(#[from] WriteError),
    #[error("{} cannot be written: {source}", path.display())]
    Unwritable { path: PathBuf, source: io::Error },
}

/// `main` prints an error it returns with `Debug`: the message, not the
/// variant's fields.
impl fmt::Debug for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
