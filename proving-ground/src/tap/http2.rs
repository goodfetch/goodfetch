//! An HTTP/2 connection's frames (RFC 9113), read after the client's
//! preface: the client's first `SETTINGS` and connection `WINDOW_UPDATE`, each
//! request's header block decoded in the order sent (HPACK, RFC 7541), each
//! answer's status, and every frame's bytes counted to its stream's request,
//! `DATA` payloads as its body.
//! Only header blocks and the frames that open the connection are kept in
//! memory; every other payload is counted and skipped.

use std::collections::BTreeMap;
use std::sync::Arc;

use loona_hpack::Decoder;

use super::text;
use crate::observation::{
    ByteCount, Connection, Http2Connection, Http2Stream, Part, Protocol, RequestHead, RequestId,
};

const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const SETTINGS: u8 = 0x4;
const WINDOW_UPDATE: u8 = 0x8;
const CONTINUATION: u8 = 0x9;

const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIORITY: u8 = 0x20;
const ACK: u8 = 0x1;

const HEADER_TABLE_SIZE: u16 = 0x1;

/// The dynamic table HPACK starts with (RFC 7541, section 4.2), which is
/// hyper's and every client's default.
const DEFAULT_TABLE_SIZE: usize = 4096;

/// The largest frame payload kept; a larger header block stops the parser.
const KEPT_LIMIT: usize = 1024 * 1024;

/// Both directions of one HTTP/2 connection.
pub(super) struct Exchange {
    incoming: Frames,
    outgoing: Frames,
    /// The client's header blocks.
    client_decoder: Decoder<'static>,
    /// The server's header blocks, for each answer's status.
    server_decoder: Decoder<'static>,
    streams: BTreeMap<u32, Stream>,
    settings: Option<Vec<(u16, u32)>>,
    window_update: Option<u32>,
    /// The settings and window update, fixed at the first request.
    http2: Option<Arc<Http2Connection>>,
    /// A header block awaiting its CONTINUATION frames, in each direction.
    incoming_block: Option<(u32, Vec<u8>)>,
    outgoing_block: Option<(u32, Vec<u8>)>,
    /// Set once a header block cannot be decoded: HPACK's state is lost,
    /// so no later block is read, and frames are only counted.
    lost: bool,
}

/// A stream's request, or its bytes until the request is known.
#[derive(Default)]
struct Stream {
    request: Option<RequestId>,
    opened: bool,
    answered: bool,
    /// Its framing's bytes and its body's, until the request is known.
    pending: ByteCount,
    pending_body: ByteCount,
}

/// One direction's frames as they arrive.
#[derive(Default)]
struct Frames {
    /// The frame header so far.
    header: Vec<u8>,
    /// The frame being read, once its header is complete.
    current: Option<Frame>,
}

struct Frame {
    kind: u8,
    flags: u8,
    stream: u32,
    left: usize,
    /// The payload, for the frames that are read.
    kept: Option<Vec<u8>>,
}

/// A complete frame whose payload was kept.
struct Kept {
    kind: u8,
    flags: u8,
    stream: u32,
    payload: Vec<u8>,
}

impl Frames {
    /// Reads `bytes`: counts each frame's bytes to its stream through
    /// `count`, a `DATA` frame's payload as [`Part::Body`], and returns each
    /// kept frame once complete.
    fn read(&mut self, mut bytes: &[u8], mut count: impl FnMut(u32, usize, Part)) -> Vec<Kept> {
        let mut kept = Vec::new();
        while !bytes.is_empty() {
            let Some(frame) = &mut self.current else {
                let needed = 9 - self.header.len();
                let (taken, rest) = bytes.split_at(needed.min(bytes.len()));
                self.header.extend_from_slice(taken);
                bytes = rest;
                if self.header.len() == 9 {
                    let header = std::mem::take(&mut self.header);
                    let length = usize::from(header[0]) << 16
                        | usize::from(header[1]) << 8
                        | usize::from(header[2]);
                    let kind = header[3];
                    let stream = u32::from_be_bytes([header[5], header[6], header[7], header[8]])
                        & 0x7fff_ffff;
                    count(stream, 9, Part::Framing);
                    let keep = matches!(kind, HEADERS | CONTINUATION)
                        || (stream == 0 && matches!(kind, SETTINGS | WINDOW_UPDATE));
                    self.current = Some(Frame {
                        kind,
                        flags: header[4],
                        stream,
                        left: length,
                        kept: (keep && length <= KEPT_LIMIT).then(Vec::new),
                    });
                    self.finish(&mut kept);
                }
                continue;
            };
            let (taken, rest) = bytes.split_at(frame.left.min(bytes.len()));
            bytes = rest;
            frame.left -= taken.len();
            let part = if frame.kind == DATA {
                Part::Body
            } else {
                Part::Framing
            };
            count(frame.stream, taken.len(), part);
            if let Some(payload) = &mut frame.kept {
                payload.extend_from_slice(taken);
            }
            self.finish(&mut kept);
        }
        kept
    }

    /// Ends the current frame if it has all its bytes.
    fn finish(&mut self, kept: &mut Vec<Kept>) {
        if self.current.as_ref().is_some_and(|frame| frame.left == 0) {
            let frame = self.current.take().expect("invariant: checked above");
            if let Some(payload) = frame.kept {
                kept.push(Kept {
                    kind: frame.kind,
                    flags: frame.flags,
                    stream: frame.stream,
                    payload,
                });
            }
        }
    }
}

impl Exchange {
    pub(super) fn new() -> Self {
        let mut client_decoder = Decoder::new();
        client_decoder.set_max_allowed_table_size(DEFAULT_TABLE_SIZE);
        let mut server_decoder = Decoder::new();
        server_decoder.set_max_allowed_table_size(DEFAULT_TABLE_SIZE);
        Self {
            incoming: Frames::default(),
            outgoing: Frames::default(),
            client_decoder,
            server_decoder,
            streams: BTreeMap::new(),
            settings: None,
            window_update: None,
            http2: None,
            incoming_block: None,
            outgoing_block: None,
            lost: false,
        }
    }

    pub(super) fn received(&mut self, connection: &Connection, bytes: &[u8]) {
        let streams = &mut self.streams;
        let frames = self.incoming.read(bytes, |stream, length, part| {
            add(connection, streams, stream, length, true, part);
        });
        for frame in frames {
            match frame.kind {
                SETTINGS if frame.flags & ACK == 0 && self.settings.is_none() => {
                    let settings: Vec<(u16, u32)> = frame
                        .payload
                        .as_chunks::<6>()
                        .0
                        .iter()
                        .map(|entry| {
                            (
                                u16::from_be_bytes([entry[0], entry[1]]),
                                u32::from_be_bytes([entry[2], entry[3], entry[4], entry[5]]),
                            )
                        })
                        .collect();
                    // The server's encoder may grow its table to what the
                    // client allows.
                    if let Some((_, size)) =
                        settings.iter().find(|(id, _)| *id == HEADER_TABLE_SIZE)
                    {
                        self.server_decoder.set_max_allowed_table_size(
                            usize::try_from(*size).unwrap_or(usize::MAX),
                        );
                    }
                    self.settings = Some(settings);
                }
                WINDOW_UPDATE if self.http2.is_none() && self.window_update.is_none() => {
                    if let Ok(increment) = <[u8; 4]>::try_from(frame.payload.as_slice()) {
                        self.window_update = Some(u32::from_be_bytes(increment) & 0x7fff_ffff);
                    }
                }
                HEADERS | CONTINUATION => {
                    if let Some(block) = block(&mut self.incoming_block, &frame) {
                        self.request(connection, frame.stream, &block);
                    }
                }
                _ => {}
            }
        }
    }

    /// Records the request a client's header block opens; a later block on
    /// the same stream (trailers) is decoded and left out.
    fn request(&mut self, connection: &Connection, stream: u32, block: &[u8]) {
        if self.lost {
            return;
        }
        let mut fields = Vec::new();
        let decoded = self.client_decoder.decode_with_cb(block, |name, value| {
            fields.push((text(&name), text(&value)));
        });
        if decoded.is_err() {
            self.lost = true;
            return;
        }
        let entry = self.streams.entry(stream).or_default();
        if entry.opened {
            return;
        }
        entry.opened = true;
        let pseudo = |wanted: &str| {
            fields
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.clone())
        };
        let (method, target) = (pseudo(":method"), pseudo(":path"));
        let authority = pseudo(":authority")
            .or_else(|| {
                fields
                    .iter()
                    .find(|(name, _)| name == "host")
                    .map(|(_, value)| value.clone())
            })
            .unwrap_or_default();
        let (pseudo_headers, headers): (Vec<_>, Vec<_>) = fields
            .into_iter()
            .partition(|(name, _)| name.starts_with(':'));
        let http2 = Arc::clone(self.http2.get_or_insert_with(|| {
            Arc::new(Http2Connection {
                settings: self.settings.clone().unwrap_or_default(),
                window_update: self.window_update,
            })
        }));
        let request = connection.request(RequestHead {
            protocol: Protocol::Http2,
            method: method.unwrap_or_default(),
            target: target.unwrap_or_default(),
            authority,
            pseudo_header_order: Some(pseudo_headers.into_iter().map(|(name, _)| name).collect()),
            headers,
            http2: Some(Http2Stream {
                connection: http2,
                stream,
            }),
        });
        let entry = self.streams.entry(stream).or_default();
        entry.request = request;
        if let Some(request) = request {
            connection.request_bytes(request, std::mem::take(&mut entry.pending), Part::Framing);
            connection.request_bytes(request, std::mem::take(&mut entry.pending_body), Part::Body);
        }
    }

    pub(super) fn sent(&mut self, connection: &Connection, bytes: &[u8]) {
        let streams = &mut self.streams;
        let frames = self.outgoing.read(bytes, |stream, length, part| {
            add(connection, streams, stream, length, false, part);
        });
        for frame in frames {
            if matches!(frame.kind, HEADERS | CONTINUATION)
                && let Some(block) = block(&mut self.outgoing_block, &frame)
            {
                self.answer(connection, frame.stream, &block);
            }
        }
    }

    /// Records the status of a stream's final answer.
    fn answer(&mut self, connection: &Connection, stream: u32, block: &[u8]) {
        if self.lost {
            return;
        }
        let mut status = None;
        let decoded = self.server_decoder.decode_with_cb(block, |name, value| {
            if &*name == b":status" {
                status = text(&value).parse::<u16>().ok();
            }
        });
        if decoded.is_err() {
            self.lost = true;
            return;
        }
        let entry = self.streams.entry(stream).or_default();
        if let Some(status) = status.filter(|status| *status >= 200)
            && !entry.answered
        {
            entry.answered = true;
            if let Some(request) = entry.request {
                connection.answered(request, status);
            }
        }
    }
}

/// Adds a frame's header block fragment to `pending`, and returns the whole
/// block once its last frame has come.
fn block(pending: &mut Option<(u32, Vec<u8>)>, frame: &Kept) -> Option<Vec<u8>> {
    if frame.kind == HEADERS {
        let mut payload = frame.payload.as_slice();
        let mut padding = 0;
        if frame.flags & PADDED != 0 {
            padding = usize::from(*payload.first()?);
            payload = &payload[1..];
        }
        if frame.flags & PRIORITY != 0 {
            payload = payload.get(5..)?;
        }
        let fragment = payload.get(..payload.len().checked_sub(padding)?)?;
        *pending = Some((frame.stream, fragment.to_vec()));
    } else {
        let (stream, block) = pending.as_mut()?;
        if *stream != frame.stream {
            return None;
        }
        block.extend_from_slice(&frame.payload);
    }
    if frame.flags & END_HEADERS == 0 {
        return None;
    }
    pending.take().map(|(_, block)| block)
}

/// Counts a frame's bytes to its stream's request, or keeps them until the
/// request is known; the connection's own frames (stream 0) are left to
/// the host's count.
fn add(
    connection: &Connection,
    streams: &mut BTreeMap<u32, Stream>,
    stream: u32,
    length: usize,
    received: bool,
    part: Part,
) {
    if stream == 0 || length == 0 {
        return;
    }
    let length = length as u64;
    let bytes = if received {
        ByteCount {
            received: length,
            sent: 0,
        }
    } else {
        ByteCount {
            received: 0,
            sent: length,
        }
    };
    let entry = streams.entry(stream).or_default();
    match entry.request {
        Some(request) => connection.request_bytes(request, bytes, part),
        None if !entry.opened => {
            let pending = match part {
                Part::Framing => &mut entry.pending,
                Part::Body => &mut entry.pending_body,
            };
            pending.received += bytes.received;
            pending.sent += bytes.sent;
        }
        None => {}
    }
}
