//! The bytes of the messages a Connection carries, which the composition's
//! handlers, the Client, the log and the protocol server share: each line
//! the engine sends, encoded once from its values where they are, and each
//! value a Client sent, read from the line it came on (ADR 0140's E1).
//! It imports no module of the engine's, so each of them may import it.

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

use goodfetch_protocol::jsonrpc::{RequestId, Version};
use serde::de::{self, DeserializeOwned, DeserializeSeed, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;

/// One line a Connection sends, and whether its bytes are its message's
/// one encode, which its writer counts (ADR 0140's E1): a `$/events` batch
/// only copies the events it carries, each encoded once as it was
/// published and counted there.
#[derive(Debug)]
pub(crate) struct Line {
    pub(crate) bytes: Vec<u8>,
    encoded: bool,
}

impl Line {
    /// A line whose bytes copy values encoded and counted before.
    pub(crate) fn copying(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            encoded: false,
        }
    }

    /// Whether its bytes are its message's encode.
    pub(crate) fn encoded(&self) -> bool {
        self.encoded
    }
}

impl From<Vec<u8>> for Line {
    /// A line encoded for its message.
    fn from(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            encoded: true,
        }
    }
}

/// The line of the notification `method` with `params`, encoded once.
pub(crate) fn notification_line(method: &str, params: &impl Serialize) -> Vec<u8> {
    line(&Outgoing {
        jsonrpc: Version,
        method,
        params,
    })
}

/// The line of the notification `method` with `params`, encoded once into a
/// buffer that starts with room for `capacity` bytes, or the error of params
/// that do not encode.
pub(crate) fn try_notification_line(
    method: &str,
    params: &impl Serialize,
    capacity: usize,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = Vec::with_capacity(capacity);
    serde_json::to_writer(
        &mut line,
        &Outgoing {
            jsonrpc: Version,
            method,
            params,
        },
    )?;
    line.push(b'\n');
    Ok(line)
}

/// A notification the engine sends, its params encoded where they are.
#[derive(Serialize)]
struct Outgoing<'a, P> {
    jsonrpc: Version,
    method: &'a str,
    params: &'a P,
}

/// A handler request the engine sends, its params encoded where they are.
#[derive(Serialize)]
struct Request<'a, P> {
    jsonrpc: Version,
    id: &'a RequestId,
    method: &'a str,
    params: &'a P,
}

/// The line of the handler request `method` with `params`, sent under `id`: a
/// function of its own, never inlined in the build `xtask counts` reads under
/// Callgrind (`--cfg goodfetch_callgrind_names`), so that the run can name it
/// in `E1.handled_page_wire_share` (ADR 0140).
#[cfg_attr(goodfetch_callgrind_names, inline(never))]
pub(crate) fn request_line(
    id: &RequestId,
    method: &str,
    params: &impl Serialize,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = serde_json::to_vec(&Request {
        jsonrpc: Version,
        id,
        method,
        params,
    })?;
    line.push(b'\n');
    Ok(line)
}

/// A success the engine sends, its result encoded where it is.
#[derive(Serialize)]
struct Success<'a, R> {
    jsonrpc: Version,
    id: &'a RequestId,
    result: &'a R,
}

/// The line of the success of the request `id`, its `result` encoded straight
/// into it: a function of its own, never inlined in the build `xtask counts`
/// reads under Callgrind (`--cfg goodfetch_callgrind_names`), so that the run
/// can name it in `E1.handled_page_wire_share` (ADR 0140).
#[cfg_attr(goodfetch_callgrind_names, inline(never))]
pub(crate) fn success_line(
    id: &RequestId,
    result: &impl Serialize,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut line = serde_json::to_vec(&Success {
        jsonrpc: Version,
        id,
        result,
    })?;
    line.push(b'\n');
    Ok(line)
}

/// The line of `message`, a message of JSON values.
pub(crate) fn line(message: &impl Serialize) -> Vec<u8> {
    let mut line =
        serde_json::to_vec(message).expect("invariant: a message of JSON values always encodes");
    line.push(b'\n');
    line
}

/// A JSON value a Client sent, kept in the line it came on and read from
/// there: an input notification's params, or a reply's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Received {
    line: String,
    range: Range<usize>,
}

impl Received {
    /// The value at `range` in `line`.
    pub(crate) fn new(line: String, range: Range<usize>) -> Self {
        Self { line, range }
    }

    /// Its JSON text.
    pub fn json(&self) -> &str {
        &self.line[self.range.clone()]
    }

    /// It, read as `T`: a function of its own, never inlined in the build
    /// `xtask counts` reads under Callgrind
    /// (`--cfg goodfetch_callgrind_names`), so that the run can name it in
    /// `E1.handled_page_wire_share` (ADR 0140).
    ///
    /// # Errors
    ///
    /// When it is no `T`.
    #[cfg_attr(goodfetch_callgrind_names, inline(never))]
    pub fn read<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(self.json())
    }
}

/// The string a JSON value holds: where it is, when it holds no escape.
pub(crate) fn text(value: &RawValue) -> Option<Cow<'_, str>> {
    let json = value.get();
    match json
        .strip_prefix('"')
        .and_then(|json| json.strip_suffix('"'))
    {
        Some(inner) if !inner.contains('\\') => Some(Cow::Borrowed(inner)),
        Some(_) => serde_json::from_str::<String>(json).ok().map(Cow::Owned),
        None => None,
    }
}

/// An object's key, where it is when it holds no escape.
pub(crate) struct Key<'a>(pub(crate) Cow<'a, str>);

impl<'de> Deserialize<'de> for Key<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(KeySeed)
    }
}

/// Reads a key, borrowed from the line when it can be.
pub(crate) struct KeySeed;

impl<'de> DeserializeSeed<'de> for KeySeed {
    type Value = Key<'de>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_str(self)
    }
}

impl<'de> Visitor<'de> for KeySeed {
    type Value = Key<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a key")
    }

    fn visit_borrowed_str<E: de::Error>(self, key: &'de str) -> Result<Self::Value, E> {
        Ok(Key(Cow::Borrowed(key)))
    }

    fn visit_str<E: de::Error>(self, key: &str) -> Result<Self::Value, E> {
        Ok(Key(Cow::Owned(key.to_owned())))
    }

    fn visit_string<E: de::Error>(self, key: String) -> Result<Self::Value, E> {
        Ok(Key(Cow::Owned(key)))
    }
}
