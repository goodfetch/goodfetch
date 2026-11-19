//! A library serving both Products (story 11-180): its helper takes the
//! client crate's `Connection`, and its `match` on the client crate's
//! `Error` catches what a program's connection fails with. Both hold only
//! while the program and the library share one copy of the client crate.

use goodfetch_collie_client::{Connection, Error};
use goodfetch_protocol::run::RunReport;

/// Starts one Run on `connection` and finishes it, or answers `None` once
/// the program has closed the connection.
///
/// # Errors
///
/// What starting or finishing the Run fails with, but `closed`.
pub async fn finish_one_run(connection: &impl Connection) -> Result<Option<RunReport>, Error> {
    match connection.start().await {
        Ok(run) => run.finish().await.map(Some),
        Err(Error::Closed) => Ok(None),
        Err(error) => Err(error),
    }
}
