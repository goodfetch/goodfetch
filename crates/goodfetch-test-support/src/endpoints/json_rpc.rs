//! One attempt on a newline-delimited JSON-RPC stream listener (ADR 0024):
//! the runner sends a first message and records whether the listener
//! replied with a `result`, closed the connection, or stayed silent.

use std::io::{BufRead as _, BufReader, ErrorKind, Write as _};

use serde_json::Value;

use super::connection::{Connection, ListenerAddress, ListenerTls, is_closed, is_silent};
use super::runner::Observed;

/// Opens a connection to `address`, over `tls` if any, sends `message` as
/// its first line, and reads until the listener replies with a `result`,
/// closes, or stays silent past the deadline. JSON-RPC errors sent before
/// closing are no reply.
pub(super) fn attempt(
    address: &ListenerAddress,
    tls: Option<&ListenerTls>,
    message: &Value,
) -> Observed {
    let mut connection = match Connection::open(address, tls) {
        Ok(connection) => connection,
        Err(error) => {
            return Observed::Unreachable {
                error: error.kind(),
            };
        }
    };
    let mut line = message.to_string();
    line.push('\n');
    if let Err(error) = connection.write_all(line.as_bytes()) {
        return closed_or_unreachable(&error);
    }
    let mut reader = BufReader::new(connection);
    loop {
        let mut answer = String::new();
        match reader.read_line(&mut answer) {
            Ok(0) => return Observed::Closed,
            Ok(_) => {
                let replied = serde_json::from_str::<Value>(&answer)
                    .is_ok_and(|answer| answer.get("result").is_some());
                if replied {
                    return Observed::Replied;
                }
            }
            Err(error) if is_silent(&error) => return Observed::StayedOpen,
            Err(error) if error.kind() == ErrorKind::InvalidData => return Observed::Malformed,
            Err(error) => return closed_or_unreachable(&error),
        }
    }
}

fn closed_or_unreachable(error: &std::io::Error) -> Observed {
    if is_closed(error) {
        Observed::Closed
    } else {
        Observed::Unreachable {
            error: error.kind(),
        }
    }
}
