//! The Connection's framing: one JSON-RPC message as one line of the
//! newline-delimited JSON every transport carries, the one encoding the
//! Connection's calls and its answers to the engine's requests both write.

use serde::Serialize;

/// `message` as one line, its newline included.
pub(crate) fn encode(message: &impl Serialize) -> serde_json::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    Ok(line)
}
