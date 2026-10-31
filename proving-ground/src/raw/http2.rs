//! The raw server's HTTP/2 (RFC 9113): the client's preface and frames read
//! one at a time, each request answered once its stream's last frame
//! arrives, in turn. Header blocks are coded with HPACK (RFC 7541).

use std::collections::BTreeMap;
use std::sync::Arc;

use loona_hpack::{Decoder, Encoder};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::{
    Answer, Context, Cut, ErrorCode, LATE, Page, Pages, READ_LIMIT, Refusal, Request, settle,
};
use crate::observation::{End, Protocol};

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const RST_STREAM: u8 = 0x3;
const SETTINGS: u8 = 0x4;
const PING: u8 = 0x6;
const GOAWAY: u8 = 0x7;
const WINDOW_UPDATE: u8 = 0x8;
const CONTINUATION: u8 = 0x9;

const END_STREAM: u8 = 0x1;
const ACK: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIORITY: u8 = 0x20;

/// The largest frame payload either side sends by default.
const FRAME_SIZE: usize = 16_384;

/// A frame as read.
struct Frame {
    kind: u8,
    flags: u8,
    stream: u32,
    payload: Vec<u8>,
}

/// A stream whose request is still arriving.
#[derive(Default)]
struct Open {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Open {
    /// The request of `stream`, the connection's `earlier`-th, now complete.
    fn request(self, stream: u32, earlier: usize, context: &Context) -> Request {
        let pseudo = |name: &str| {
            self.headers
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        Request {
            protocol: Protocol::Http2,
            method: pseudo(":method"),
            target: pseudo(":path"),
            authority: pseudo(":authority"),
            headers: self
                .headers
                .iter()
                .filter(|(name, _)| !name.starts_with(':'))
                .cloned()
                .collect(),
            body: self.body,
            server_name: context.server_name.clone(),
            earlier_on_connection: earlier,
            recorded: context.observed.latest_request(stream),
            observed: Arc::clone(&context.observed),
        }
    }
}

/// One connection's state.
struct Connection<S> {
    stream: S,
    decoder: Decoder<'static>,
    encoder: Encoder<'static>,
    /// The last stream answered or refused, which a `GOAWAY` names.
    last_processed: u32,
    requests: usize,
}

/// Serves the connection's streams until the client or an answer ends it.
pub(super) async fn serve<S, P>(stream: S, context: &Context, pages: &P)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    P: Pages,
{
    let mut connection = Connection {
        stream,
        decoder: Decoder::new(),
        encoder: Encoder::new(),
        last_processed: 0,
        requests: 0,
    };
    let _ = connection.run(context, pages).await;
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> Connection<S> {
    async fn run<P: Pages>(&mut self, context: &Context, pages: &P) -> std::io::Result<()> {
        let mut preface = [0; PREFACE.len()];
        self.stream.read_exact(&mut preface).await?;
        if preface != PREFACE {
            return Ok(());
        }
        self.frame(SETTINGS, 0, 0, &[]).await?;
        let mut open: BTreeMap<u32, Open> = BTreeMap::new();
        let mut block: Option<(u32, u8, Vec<u8>)> = None;
        loop {
            let frame = self.read_frame().await?;
            match frame.kind {
                SETTINGS if frame.flags & ACK == 0 => self.frame(SETTINGS, ACK, 0, &[]).await?,
                PING if frame.flags & ACK == 0 => {
                    self.frame(PING, ACK, 0, &frame.payload).await?;
                }
                GOAWAY => return Ok(()),
                RST_STREAM => {
                    open.remove(&frame.stream);
                }
                HEADERS => {
                    let fragment = headers_fragment(&frame)?;
                    if frame.flags & END_HEADERS == 0 {
                        block = Some((frame.stream, frame.flags, fragment));
                        continue;
                    }
                    let end = self.headers(&mut open, frame.stream, frame.flags, &fragment)?;
                    if end
                        && self
                            .complete(&mut open, frame.stream, context, pages)
                            .await?
                    {
                        return Ok(());
                    }
                }
                CONTINUATION => {
                    let Some((stream, flags, mut fragment)) = block.take() else {
                        return Ok(());
                    };
                    fragment.extend_from_slice(&frame.payload);
                    if fragment.len() > READ_LIMIT {
                        return Ok(());
                    }
                    if frame.flags & END_HEADERS == 0 {
                        block = Some((stream, flags, fragment));
                        continue;
                    }
                    let end = self.headers(&mut open, stream, flags, &fragment)?;
                    if end && self.complete(&mut open, stream, context, pages).await? {
                        return Ok(());
                    }
                }
                DATA => {
                    let data = data_payload(&frame)?;
                    if !frame.payload.is_empty() {
                        // Gives back what the frame took of both windows.
                        let increment = u32::try_from(frame.payload.len())
                            .expect("invariant: a frame is shorter than 2^24 bytes")
                            .to_be_bytes();
                        self.frame(WINDOW_UPDATE, 0, 0, &increment).await?;
                        self.frame(WINDOW_UPDATE, 0, frame.stream, &increment)
                            .await?;
                    }
                    if let Some(request) = open.get_mut(&frame.stream) {
                        request.body.extend_from_slice(data);
                        if request.body.len() > READ_LIMIT {
                            return Ok(());
                        }
                    }
                    if frame.flags & END_STREAM != 0
                        && self
                            .complete(&mut open, frame.stream, context, pages)
                            .await?
                    {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }

    /// Decodes a header block into its stream's request; whether the
    /// request ended with it.
    fn headers(
        &mut self,
        open: &mut BTreeMap<u32, Open>,
        stream: u32,
        flags: u8,
        block: &[u8],
    ) -> std::io::Result<bool> {
        let fields = self
            .decoder
            .decode(block)
            .map_err(|_| std::io::Error::other("an undecodable header block"))?;
        let request = open.entry(stream).or_default();
        request
            .headers
            .extend(fields.into_iter().map(|(name, value)| {
                (
                    String::from_utf8_lossy(&name).into_owned(),
                    String::from_utf8_lossy(&value).into_owned(),
                )
            }));
        Ok(flags & END_STREAM != 0)
    }

    /// Answers the request of `stream`, now complete; whether the connection
    /// ends with it.
    async fn complete<P: Pages>(
        &mut self,
        open: &mut BTreeMap<u32, Open>,
        stream: u32,
        context: &Context,
        pages: &P,
    ) -> std::io::Result<bool> {
        let Some(arrived) = open.remove(&stream) else {
            return Ok(false);
        };
        let request = arrived.request(stream, self.requests, context);
        self.requests += 1;
        // A filler body is past the window the server never waits beyond,
        // and a stalled one is HTTP/1.1's alone.
        let answer = match settle(pages.answer(&request).await).await {
            Answer::Filler { .. } | Answer::Stalled { .. } => {
                Answer::Refuse(Refusal::Reset(ErrorCode::InternalError))
            }
            answer => answer,
        };
        match answer {
            Answer::Page(page) => {
                self.page(stream, &page, page.body.len()).await?;
                self.last_processed = stream;
                context.delivered(
                    request.recorded,
                    Some(page.body.len()),
                    page.body.len(),
                    End::Complete,
                );
                Ok(false)
            }
            Answer::Cut { page, sent, cut } => {
                let sent = sent.min(page.body.len());
                self.page(stream, &page, sent).await?;
                self.last_processed = stream;
                match cut {
                    Cut::Stream => {
                        let code = ErrorCode::InternalError;
                        self.frame(RST_STREAM, 0, stream, &code.value().to_be_bytes())
                            .await?;
                        context.delivered(
                            request.recorded,
                            Some(page.body.len()),
                            sent,
                            End::StreamReset {
                                error_code: code.name(),
                            },
                        );
                        Ok(false)
                    }
                    Cut::Connection | Cut::Late => {
                        if cut == Cut::Late {
                            tokio::time::sleep(LATE).await;
                        }
                        context.delivered(
                            request.recorded,
                            Some(page.body.len()),
                            sent,
                            End::ConnectionClosed,
                        );
                        Ok(true)
                    }
                }
            }
            Answer::Refuse(Refusal::Reset(code)) => {
                self.frame(RST_STREAM, 0, stream, &code.value().to_be_bytes())
                    .await?;
                self.last_processed = stream;
                context.delivered(
                    request.recorded,
                    None,
                    0,
                    End::StreamReset {
                        error_code: code.name(),
                    },
                );
                Ok(false)
            }
            Answer::Refuse(Refusal::GoAway) => {
                let code = ErrorCode::NoError;
                let mut payload = self.last_processed.to_be_bytes().to_vec();
                payload.extend_from_slice(&code.value().to_be_bytes());
                self.frame(GOAWAY, 0, 0, &payload).await?;
                context.delivered(
                    request.recorded,
                    None,
                    0,
                    End::GoAway {
                        last_stream: self.last_processed,
                        error_code: code.name(),
                    },
                );
                let _ = self.stream.shutdown().await;
                Ok(true)
            }
            Answer::Refuse(Refusal::Close) => {
                context.delivered(request.recorded, None, 0, End::ConnectionClosed);
                Ok(true)
            }
            Answer::Filler { .. } | Answer::Stalled { .. } | Answer::Later(..) => {
                unreachable!(
                    "invariant: a filler or stalled body is reset above, and `settle` resolves every wait"
                )
            }
        }
    }

    /// Sends a page's head and the first `sent` bytes of its body, ending
    /// the stream when they are the whole body.
    async fn page(&mut self, stream: u32, page: &Page, sent: usize) -> std::io::Result<()> {
        let status = page.status.to_string();
        let length = page.body.len().to_string();
        let mut fields: Vec<(&[u8], &[u8])> = vec![(b":status", status.as_bytes())];
        fields.extend(
            page.headers
                .iter()
                .filter(|(name, _)| *name != "connection")
                .map(|(name, value)| (name.as_bytes(), value.as_bytes())),
        );
        fields.push((b"content-length", length.as_bytes()));
        let block = self.encoder.encode(fields);
        let whole = sent == page.body.len();
        let body = &page.body[..sent];
        let flags = END_HEADERS
            | if whole && body.is_empty() {
                END_STREAM
            } else {
                0
            };
        self.frame(HEADERS, flags, stream, &block).await?;
        let mut chunks = body.chunks(FRAME_SIZE).peekable();
        while let Some(chunk) = chunks.next() {
            let last = chunks.peek().is_none();
            let flags = if last && whole { END_STREAM } else { 0 };
            self.frame(DATA, flags, stream, chunk).await?;
        }
        Ok(())
    }

    async fn read_frame(&mut self) -> std::io::Result<Frame> {
        let mut header = [0; 9];
        self.stream.read_exact(&mut header).await?;
        let length =
            usize::from(header[0]) << 16 | usize::from(header[1]) << 8 | usize::from(header[2]);
        if length > READ_LIMIT {
            return Err(std::io::Error::other("a frame past the read limit"));
        }
        let mut payload = vec![0; length];
        self.stream.read_exact(&mut payload).await?;
        Ok(Frame {
            kind: header[3],
            flags: header[4],
            stream: u32::from_be_bytes([header[5], header[6], header[7], header[8]]) & 0x7fff_ffff,
            payload,
        })
    }

    async fn frame(
        &mut self,
        kind: u8,
        flags: u8,
        stream: u32,
        payload: &[u8],
    ) -> std::io::Result<()> {
        let length = u32::try_from(payload.len())
            .expect("invariant: a frame is shorter than 2^24 bytes")
            .to_be_bytes();
        let mut bytes = Vec::with_capacity(9 + payload.len());
        bytes.extend_from_slice(&length[1..]);
        bytes.push(kind);
        bytes.push(flags);
        bytes.extend_from_slice(&stream.to_be_bytes());
        bytes.extend_from_slice(payload);
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await
    }
}

/// A `HEADERS` frame's header block fragment, without padding or priority.
fn headers_fragment(frame: &Frame) -> std::io::Result<Vec<u8>> {
    let mut payload = unpadded(frame)?;
    if frame.flags & PRIORITY != 0 {
        payload = payload
            .get(5..)
            .ok_or_else(|| std::io::Error::other("a short HEADERS frame"))?;
    }
    Ok(payload.to_vec())
}

/// A `DATA` frame's data, without padding.
fn data_payload(frame: &Frame) -> std::io::Result<&[u8]> {
    unpadded(frame)
}

fn unpadded(frame: &Frame) -> std::io::Result<&[u8]> {
    if frame.flags & PADDED == 0 {
        return Ok(&frame.payload);
    }
    let short = || std::io::Error::other("a short padded frame");
    let padding = usize::from(*frame.payload.first().ok_or_else(short)?);
    frame
        .payload
        .get(1..frame.payload.len().checked_sub(padding).ok_or_else(short)?)
        .ok_or_else(short)
}
