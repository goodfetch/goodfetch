//! The browser's ClientHello, as the egress relay reads it before it
//! answers (ADR 0124): the TLS records that carry it, kept as they came so
//! the relay's own handshake with the browser reads them again, and the
//! values the relay sets on its upstream connection so the site sees the
//! browser's own fingerprint. GREASE values (RFC 8701) are left out of
//! every list; that the browser sent any is kept apart, since BoringSSL
//! draws its own. A tunnel the relay passes as it is notes its ClientHello
//! as the bytes go by ([`Noting`]), for the relay's own look at a
//! certificate the browser refused.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, ReadBuf};

/// The most bytes of records the relay reads for one ClientHello: a
/// browser's, with its post-quantum key share, fits in a few KiB.
const MOST: usize = 64 * 1024;

/// A TLS record of the handshake protocol.
const HANDSHAKE: u8 = 22;

/// The handshake message that opens a connection.
const CLIENT_HELLO: u8 = 1;

/// The extensions the relay reads values from (IANA's TLS `ExtensionType`
/// registry).
mod extension {
    pub(super) const SERVER_NAME: u16 = 0;
    pub(super) const STATUS_REQUEST: u16 = 5;
    pub(super) const SUPPORTED_GROUPS: u16 = 10;
    pub(super) const SIGNATURE_ALGORITHMS: u16 = 13;
    pub(super) const ALPN: u16 = 16;
    pub(super) const SIGNED_CERTIFICATE_TIMESTAMP: u16 = 18;
    pub(super) const COMPRESS_CERTIFICATE: u16 = 27;
    pub(super) const RECORD_SIZE_LIMIT: u16 = 28;
    pub(super) const DELEGATED_CREDENTIAL: u16 = 34;
    pub(super) const SESSION_TICKET: u16 = 35;
    pub(super) const SUPPORTED_VERSIONS: u16 = 43;
    pub(super) const KEY_SHARE: u16 = 51;
    pub(super) const APPLICATION_SETTINGS_OLD: u16 = 17_513;
    pub(super) const APPLICATION_SETTINGS: u16 = 17_613;
    pub(super) const ENCRYPTED_CLIENT_HELLO: u16 = 0xfe0d;
    pub(super) const TRUST_ANCHORS: u16 = 0xca34;
}

/// What the relay mirrors of a browser's ClientHello.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClientHello {
    /// Its cipher suites, in order.
    pub(crate) cipher_suites: Vec<u16>,
    /// The types of its extensions, in the order sent.
    pub(crate) extensions: Vec<u16>,
    /// Whether it carried any GREASE value.
    pub(crate) grease: bool,
    /// The host name it named (SNI).
    pub(crate) server_name: Option<String>,
    /// Its supported groups, in order.
    pub(crate) groups: Vec<u16>,
    /// The groups it sent a key share for, in order.
    pub(crate) key_shares: Vec<u16>,
    /// Its signature algorithms, in order.
    pub(crate) signature_algorithms: Vec<u16>,
    /// Its ALPN protocols, in order.
    pub(crate) alpn: Vec<Vec<u8>>,
    /// The protocols of its ALPS extension, and whether it used the new
    /// codepoint.
    pub(crate) alps: Vec<Vec<u8>>,
    pub(crate) alps_new_codepoint: bool,
    /// The certificate compression algorithms it accepts (RFC 8879).
    pub(crate) certificate_compression: Vec<u16>,
    /// The protocol versions it supports, in order.
    pub(crate) versions: Vec<u16>,
    /// Its `record_size_limit` (RFC 8449).
    pub(crate) record_size_limit: Option<u16>,
    /// The signature algorithms of its `delegated_credential` extension.
    pub(crate) delegated_credentials: Vec<u16>,
    /// The trust anchor IDs of its `trust_anchors` extension (Chromium's
    /// codepoint of draft-ietf-tls-trust-anchor-ids), each behind its
    /// one-byte length as the extension lists them.
    pub(crate) trust_anchors: Option<Vec<u8>>,
}

impl ClientHello {
    /// Whether it carried the extension of type `kind`.
    pub(crate) fn has(&self, kind: u16) -> bool {
        self.extensions.contains(&kind)
    }

    /// Whether it asked for a stapled OCSP response.
    pub(crate) fn status_request(&self) -> bool {
        self.has(extension::STATUS_REQUEST)
    }

    /// Whether it asked for signed certificate timestamps.
    pub(crate) fn signed_certificate_timestamps(&self) -> bool {
        self.has(extension::SIGNED_CERTIFICATE_TIMESTAMP)
    }

    /// Whether it offered session tickets.
    pub(crate) fn session_ticket(&self) -> bool {
        self.has(extension::SESSION_TICKET)
    }

    /// Whether it sent an ECH extension, which a browser with no ECH
    /// configuration for the site sends as GREASE.
    pub(crate) fn ech_grease(&self) -> bool {
        self.has(extension::ENCRYPTED_CLIENT_HELLO)
    }
}

/// Why no ClientHello was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// The connection closed first.
    Closed,
    /// What came is no ClientHello.
    Malformed,
}

/// Reads the records that carry the browser's ClientHello from `stream`:
/// the ClientHello, and every byte read, which its handshake with the
/// relay reads again.
///
/// # Errors
///
/// [`Unread`] when the stream closes first or sends something else.
///
/// # Cancel safety
///
/// Dropping the future loses what it read.
pub(crate) async fn read(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<(ClientHello, Vec<u8>), Unread> {
    let mut raw = Vec::new();
    let mut message = Vec::new();
    loop {
        let mut header = [0_u8; 5];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|_| Unread::Closed)?;
        if header[0] != HANDSHAKE {
            return Err(Unread::Malformed);
        }
        let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
        if raw.len() + 5 + length > MOST {
            return Err(Unread::Malformed);
        }
        let mut body = vec![0_u8; length];
        stream
            .read_exact(&mut body)
            .await
            .map_err(|_| Unread::Closed)?;
        raw.extend_from_slice(&header);
        raw.extend_from_slice(&body);
        message.extend_from_slice(&body);
        if let Some(whole) = whole_message(&message)? {
            return Ok((parse_message(whole)?, raw));
        }
    }
}

/// The ClientHello message `message` holds once it holds all of it.
fn whole_message(message: &[u8]) -> Result<Option<&[u8]>, Unread> {
    let Some(&[kind, a, b, c]) = message.get(..4) else {
        return Ok(None);
    };
    if kind != CLIENT_HELLO {
        return Err(Unread::Malformed);
    }
    let length = usize::from(a) << 16 | usize::from(b) << 8 | usize::from(c);
    Ok(message.get(4..4 + length))
}

/// Parses `records`, the bytes of the TLS records that carry a
/// ClientHello.
///
/// # Errors
///
/// [`Unread::Malformed`] for anything else, [`Unread::Closed`] for too few
/// bytes.
pub(crate) fn parse(records: &[u8]) -> Result<ClientHello, Unread> {
    let mut rest = records;
    let mut message = Vec::new();
    loop {
        let mut record = Reader(rest);
        if record.u8().ok_or(Unread::Closed)? != HANDSHAKE {
            return Err(Unread::Malformed);
        }
        record.take(2).ok_or(Unread::Closed)?;
        let body = record.vector16().ok_or(Unread::Closed)?;
        message.extend_from_slice(body);
        rest = record.0;
        if let Some(whole) = whole_message(&message)? {
            return parse_message(whole);
        }
    }
}

/// A browser's stream whose first bytes, read as they pass unchanged, give
/// the ClientHello they carry to its `noted`, once, so a tunnel the relay
/// passes as it is still tells how the browser opens TLS. Bytes of another
/// protocol, or more than a ClientHello's records hold, end the noting;
/// nothing ever waits on it.
pub(crate) struct Noting<B, F> {
    inner: B,
    /// The bytes read so far, while no ClientHello is whole.
    seen: Vec<u8>,
    /// Takes the ClientHello once it is whole; none once noting ended.
    noted: Option<F>,
}

impl<B, F: FnOnce(ClientHello)> Noting<B, F> {
    /// Notes the ClientHello of `inner`'s bytes, `first`, the bytes the
    /// relay already read past the browser's head, before them.
    pub(crate) fn new(inner: B, first: &[u8], noted: F) -> Self {
        let mut noting = Self {
            inner,
            seen: Vec::new(),
            noted: Some(noted),
        };
        noting.note(first);
        noting
    }

    fn note(&mut self, bytes: &[u8]) {
        if self.noted.is_none() || bytes.is_empty() {
            return;
        }
        self.seen.extend_from_slice(bytes);
        match parse(&self.seen) {
            Err(Unread::Closed) if self.seen.len() <= MOST => return,
            Ok(hello) => {
                if let Some(noted) = self.noted.take() {
                    noted(hello);
                }
            }
            Err(_) => {}
        }
        self.noted = None;
        self.seen = Vec::new();
    }
}

impl<B: AsyncRead + Unpin, F: FnOnce(ClientHello) + Unpin> AsyncRead for Noting<B, F> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        let before = buffer.filled().len();
        let polled = Pin::new(&mut this.inner).poll_read(context, buffer);
        if let Poll::Ready(Ok(())) = polled {
            this.note(&buffer.filled()[before..]);
        }
        polled
    }
}

impl<B: AsyncWrite + Unpin, F: Unpin> AsyncWrite for Noting<B, F> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

/// Parses a ClientHello's body.
fn parse_message(body: &[u8]) -> Result<ClientHello, Unread> {
    message(body).ok_or(Unread::Malformed)
}

fn message(body: &[u8]) -> Option<ClientHello> {
    let mut hello = ClientHello::default();
    let mut reader = Reader(body);
    // legacy_version, random, legacy_session_id.
    reader.take(2 + 32)?;
    reader.vector8()?;
    let mut suites = Reader(reader.vector16()?);
    while !suites.0.is_empty() {
        let suite = suites.u16()?;
        hello.keep(suite, |hello| &mut hello.cipher_suites);
    }
    reader.vector8()?;
    if reader.0.is_empty() {
        return Some(hello);
    }
    let mut extensions = Reader(reader.vector16()?);
    while !extensions.0.is_empty() {
        let kind = extensions.u16()?;
        let data = extensions.vector16()?;
        if grease(kind) {
            hello.grease = true;
            continue;
        }
        hello.extensions.push(kind);
        hello.extension(kind, data)?;
    }
    Some(hello)
}

impl ClientHello {
    /// Keeps `value` in the list `list` picks, unless it is GREASE.
    fn keep(&mut self, value: u16, list: impl FnOnce(&mut Self) -> &mut Vec<u16>) {
        if grease(value) {
            self.grease = true;
        } else {
            list(self).push(value);
        }
    }

    /// Keeps the list of 16-bit values `data` holds behind a length of
    /// `prefix` bytes.
    fn keep_list(
        &mut self,
        data: &[u8],
        prefix: usize,
        list: fn(&mut Self) -> &mut Vec<u16>,
    ) -> Option<()> {
        let mut reader = Reader(data);
        let mut values = Reader(if prefix == 1 {
            reader.vector8()?
        } else {
            reader.vector16()?
        });
        while !values.0.is_empty() {
            let value = values.u16()?;
            self.keep(value, list);
        }
        Some(())
    }

    fn extension(&mut self, kind: u16, data: &[u8]) -> Option<()> {
        match kind {
            extension::SERVER_NAME => {
                let mut list = Reader(Reader(data).vector16()?);
                while !list.0.is_empty() {
                    let name_type = list.u8()?;
                    let name = list.vector16()?;
                    if name_type == 0 && self.server_name.is_none() {
                        self.server_name = Some(String::from_utf8(name.to_vec()).ok()?);
                    }
                }
            }
            extension::SUPPORTED_GROUPS => self.keep_list(data, 2, |hello| &mut hello.groups)?,
            extension::SIGNATURE_ALGORITHMS => {
                self.keep_list(data, 2, |hello| &mut hello.signature_algorithms)?;
            }
            extension::DELEGATED_CREDENTIAL => {
                self.keep_list(data, 2, |hello| &mut hello.delegated_credentials)?;
            }
            extension::COMPRESS_CERTIFICATE => {
                self.keep_list(data, 1, |hello| &mut hello.certificate_compression)?;
            }
            extension::SUPPORTED_VERSIONS => {
                self.keep_list(data, 1, |hello| &mut hello.versions)?;
            }
            extension::ALPN => self.alpn = protocols(data)?,
            extension::APPLICATION_SETTINGS | extension::APPLICATION_SETTINGS_OLD => {
                self.alps = protocols(data)?;
                self.alps_new_codepoint = kind == extension::APPLICATION_SETTINGS;
            }
            extension::RECORD_SIZE_LIMIT => self.record_size_limit = Some(Reader(data).u16()?),
            extension::TRUST_ANCHORS => {
                self.trust_anchors = Some(Reader(data).vector16()?.to_vec());
            }
            extension::KEY_SHARE => {
                let mut shares = Reader(Reader(data).vector16()?);
                while !shares.0.is_empty() {
                    let group = shares.u16()?;
                    shares.vector16()?;
                    self.keep(group, |hello| &mut hello.key_shares);
                }
            }
            _ => {}
        }
        Some(())
    }
}

/// The protocol names of an ALPN or ALPS extension's list.
fn protocols(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut list = Reader(Reader(data).vector16()?);
    let mut names = Vec::new();
    while !list.0.is_empty() {
        names.push(list.vector8()?.to_vec());
    }
    Some(names)
}

/// Whether `value` is a GREASE value: `0x?a?a` (RFC 8701).
pub(crate) fn grease(value: u16) -> bool {
    value & 0x0f0f == 0x0a0a && value >> 8 == value & 0xff
}

/// Reads big-endian fields off the front of a slice.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.0.len() < count {
            return None;
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Some(taken)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn vector8(&mut self) -> Option<&'a [u8]> {
        let length = usize::from(self.u8()?);
        self.take(length)
    }

    fn vector16(&mut self) -> Option<&'a [u8]> {
        let length = usize::from(self.u16()?);
        self.take(length)
    }
}
