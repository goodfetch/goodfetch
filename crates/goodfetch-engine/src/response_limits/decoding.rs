//! A body decoded as it arrives, under the limits ([`Decoding`]): every
//! reader of a body from outside decodes it with this one decoder, the
//! fetch tier's driver, a sitemap's reader and the engine's own, so a body
//! gets one verdict on every HTTP path (spec #2164, issue 2187).
//!
//! Each content coding `Content-Encoding` lists is decoded, the last
//! applied first, as Chrome decodes it: `gzip` (and `x-gzip`), `deflate`
//! with its zlib wrapper or raw, as Chrome falls back to raw deflate, `br`
//! and `zstd`. A list naming `identity` or a coding Chrome does not know
//! passes the whole body through undecoded, as Chrome's `URLRequestHttpJob`
//! does. Bytes after a gzip or deflate stream's end are ignored, and bytes
//! after a Brotli stream's end do not decode.
//!
//! **Verdicts.** A body that is no stream of its coding is
//! [`BodyFailure::Corrupt`] as soon as its bytes show it; one whose stream
//! stops before its end is [`BodyFailure::Cut`] once it ended (ADR 0118);
//! and one that decodes past a limit is [`BodyFailure::Limit`] at the
//! decoder's buffer that passed it (ADR 0067). Each coding's decoded bytes
//! count against the limits over the bytes received so far, an inner
//! coding's as much as the body's, so a bomb under stacked codings stops
//! where it passes them. A zstd frame may ask for a window of at most
//! 8 MiB, RFC 9659's, and one that asks for more does not decode, as
//! Chromium refuses it (`ERR_ZSTD_WINDOW_SIZE_TOO_BIG`).
//!
//! **Kept bytes.** A body read for its first bytes alone
//! ([`Decoding::keeping`]), such as a robots.txt's 500 KiB, keeps those and
//! decodes no further: the limits judge the bytes kept, so a bomb's start
//! is read, not refused.
//!
//! Every decoder drains its input on each push, so what a body's bytes so
//! far decode to ([`Decoding::decoded`]) is there without a flush.

use flate2::{Crc, Decompress, FlushDecompress, Status};
use goodfetch_protocol::response_limits::LimitSetting;

use super::{LimitExceeded, Limits};

/// The largest window, as a power of two, a zstd frame may ask for: 8 MiB
/// (RFC 9659, and Chromium's `windowLogMax`).
const ZSTD_WINDOW_LOG_MAX: u32 = 23;

/// The bytes a decoder writes out at once.
const DECODER_BUFFER: usize = 32 * 1024;

/// The two bytes a gzip stream opens with (RFC 1952).
pub const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// The four bytes a zstd frame opens with (RFC 8878).
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// Why a body was not read whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyFailure {
    /// It passed one of the limits.
    Limit(LimitExceeded),
    /// It is no stream of the codings it names.
    Corrupt,
    /// A stream of its codings stops before its end: the body was cut
    /// (ADR 0118).
    Cut,
}

/// A body being decoded as it arrives, under the limits.
pub struct Decoding {
    /// The URL whose body it is, which a refusal names.
    url: String,
    limits: Limits,
    /// The bytes received on the wire so far.
    received: u64,
    /// One stage per coding, the last applied first.
    stages: Vec<Stage>,
    /// The decoded bytes kept.
    kept: Kept,
    /// Whether the decoding goes on.
    state: State,
}

impl std::fmt::Debug for Decoding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Decoding")
            .field("url", &self.url)
            .field("received", &self.received)
            .field("kept", &self.kept.bytes.len())
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Whether a [`Decoding`] goes on.
#[derive(Debug)]
enum State {
    /// It takes more bytes.
    Reading,
    /// It holds all the bytes it keeps: what else arrives is left.
    Full,
    /// It failed: every later push gives the same failure.
    Failed(BodyFailure),
}

impl Decoding {
    /// A body of `url` under `codings`, each a `Content-Encoding` field's
    /// value in order, read under `limits`.
    pub fn new<'a>(url: &str, codings: impl Iterator<Item = &'a str>, limits: Limits) -> Self {
        // The last coding applied is the first decoded.
        let stages = ContentCoding::listed(codings)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .map(|coding| Stage {
                decoder: Decoder::of(coding),
                produced: 0,
            })
            .collect();
        Self {
            url: url.to_owned(),
            limits,
            received: 0,
            stages,
            kept: Kept {
                bytes: Vec::new(),
                keep: None,
            },
            state: State::Reading,
        }
    }

    /// The same decoding, keeping the first `keep` decoded bytes alone:
    /// once it holds them it decodes no further, and the limits judge only
    /// them.
    #[must_use]
    pub fn keeping(mut self, keep: usize) -> Self {
        self.kept.keep = Some(keep);
        self
    }

    /// Decodes the body's next `received` bytes; once it holds all it
    /// keeps, it takes no more.
    ///
    /// # Errors
    ///
    /// [`BodyFailure::Limit`] once the decoded bytes pass a limit;
    /// [`BodyFailure::Corrupt`] once they show no stream of their coding.
    /// A failed decoding gives its failure again on each later push.
    pub fn push(&mut self, received: &[u8]) -> Result<(), BodyFailure> {
        match &self.state {
            State::Reading => {}
            State::Full => return Ok(()),
            State::Failed(failure) => return Err(failure.clone()),
        }
        self.received +=
            u64::try_from(received.len()).expect("invariant: a chunk's length fits in u64");
        let judge = Judge {
            url: &self.url,
            limits: &self.limits,
            received: self.received,
        };
        let fed = feed(&mut self.stages, &judge, &mut self.kept, received);
        self.stopped(fed)
    }

    /// Whether it holds every byte it keeps ([`Self::keeping`]).
    pub fn is_full(&self) -> bool {
        matches!(self.state, State::Full)
    }

    /// Every byte the body received so far decodes to, as far as it keeps
    /// them: enough to judge a body by its first bytes before the rest
    /// arrives.
    pub fn decoded(&self) -> &[u8] {
        &self.kept.bytes
    }

    /// The whole decoded body, once its last bytes arrived, or the bytes
    /// it keeps.
    ///
    /// # Errors
    ///
    /// As [`Self::push`] says, and [`BodyFailure::Cut`] for a stream that
    /// stops before its end.
    pub fn finish(mut self) -> Result<Vec<u8>, BodyFailure> {
        if matches!(self.state, State::Reading) {
            let judge = Judge {
                url: &self.url,
                limits: &self.limits,
                received: self.received,
            };
            let ended = end(&mut self.stages, &judge, &mut self.kept);
            self.stopped(ended)?;
        }
        match self.state {
            State::Failed(failure) => Err(failure),
            State::Reading | State::Full => Ok(self.kept.bytes),
        }
    }

    /// What a stage's `result` leaves the decoding in.
    fn stopped(&mut self, result: Result<(), Stop>) -> Result<(), BodyFailure> {
        match result {
            Ok(()) => Ok(()),
            Err(Stop::Full) => {
                self.state = State::Full;
                Ok(())
            }
            Err(Stop::Failed(failure)) => {
                self.state = State::Failed(failure.clone());
                Err(failure)
            }
        }
    }
}

/// Why a stage stopped before its input's end.
#[derive(Debug)]
enum Stop {
    /// The body failed.
    Failed(BodyFailure),
    /// Every byte kept is in.
    Full,
}

impl From<BodyFailure> for Stop {
    fn from(failure: BodyFailure) -> Self {
        Self::Failed(failure)
    }
}

/// Judges decoded bytes against the limits, over the bytes received so far.
struct Judge<'a> {
    url: &'a str,
    limits: &'a Limits,
    received: u64,
}

impl Judge<'_> {
    /// Fails once `decoded` bytes pass a limit.
    fn judge(&self, decoded: usize) -> Result<(), Stop> {
        let decoded = u64::try_from(decoded).expect("invariant: a length fits in u64");
        match self.limits.passed(decoded, Some(self.received)) {
            Some(setting) => Err(self.refused(setting)),
            None => Ok(()),
        }
    }

    fn refused(&self, setting: LimitSetting) -> Stop {
        Stop::Failed(BodyFailure::Limit(self.limits.exceeded(self.url, setting)))
    }
}

/// The decoded bytes a body keeps.
struct Kept {
    bytes: Vec<u8>,
    /// The most it keeps; `None` keeps the whole body.
    keep: Option<usize>,
}

impl Kept {
    /// Keeps `decoded`, as far as there is room, once the limits allow it.
    fn take(&mut self, judge: &Judge<'_>, decoded: &[u8]) -> Result<(), Stop> {
        let room = self.keep.map_or(decoded.len(), |keep| {
            keep.saturating_sub(self.bytes.len()).min(decoded.len())
        });
        judge.judge(self.bytes.len() + room)?;
        self.bytes.extend_from_slice(&decoded[..room]);
        if self.keep.is_some_and(|keep| self.bytes.len() >= keep) {
            return Err(Stop::Full);
        }
        Ok(())
    }
}

/// One coding's decoder, and the bytes it decoded so far.
struct Stage {
    decoder: Decoder,
    produced: usize,
}

/// Decodes `input` through `stages`, the outermost coding first, into
/// `kept`: each inner coding's bytes are judged against the limits as they
/// are decoded, whatever the body keeps of the last.
fn feed(
    stages: &mut [Stage],
    judge: &Judge<'_>,
    kept: &mut Kept,
    input: &[u8],
) -> Result<(), Stop> {
    let Some((Stage { decoder, produced }, inner)) = stages.split_first_mut() else {
        return kept.take(judge, input);
    };
    decoder.push(input, &mut |decoded| {
        passed_on(produced, inner, judge, kept, decoded)
    })
}

/// Ends each of `stages` in turn, the outermost first, passing what each
/// still gives to the next.
fn end(stages: &mut [Stage], judge: &Judge<'_>, kept: &mut Kept) -> Result<(), Stop> {
    let Some((Stage { decoder, produced }, inner)) = stages.split_first_mut() else {
        return Ok(());
    };
    decoder.end(&mut |decoded| passed_on(produced, inner, judge, kept, decoded))?;
    end(inner, judge, kept)
}

/// Passes `decoded`, a stage's output, to the `inner` stages, counted in
/// the stage's `produced` and judged when another coding decodes it again.
fn passed_on(
    produced: &mut usize,
    inner: &mut [Stage],
    judge: &Judge<'_>,
    kept: &mut Kept,
    decoded: &[u8],
) -> Result<(), Stop> {
    if decoded.is_empty() {
        return Ok(());
    }
    *produced += decoded.len();
    if !inner.is_empty() {
        judge.judge(*produced)?;
    }
    feed(inner, judge, kept, decoded)
}

/// Where a decoder writes what it decodes.
type Out<'a> = &'a mut dyn FnMut(&[u8]) -> Result<(), Stop>;

/// A content coding the engine decodes, as Chrome decodes it: every tier
/// that reads a body names its codings with this one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentCoding {
    /// `gzip`, and `x-gzip`.
    Gzip,
    /// `deflate`, with its zlib wrapper or raw ([`has_zlib_header`]).
    Deflate,
    /// `br`.
    Brotli,
    /// `zstd`.
    Zstd,
}

impl ContentCoding {
    /// The coding a `Content-Encoding` token names, `None` for `identity`
    /// and every coding Chrome does not know, which leaves the body as it
    /// is.
    pub fn named(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "gzip" | "x-gzip" => Some(Self::Gzip),
            "deflate" => Some(Self::Deflate),
            "br" => Some(Self::Brotli),
            "zstd" => Some(Self::Zstd),
            _ => None,
        }
    }

    /// The codings `fields` list, each a `Content-Encoding` field's value
    /// in order, in the order they were applied; `None` when one names
    /// `identity` or a coding Chrome does not know, which, as in Chrome's
    /// `URLRequestHttpJob`, leaves the whole body undecoded.
    pub fn listed<'a>(fields: impl Iterator<Item = &'a str>) -> Option<Vec<Self>> {
        fields
            .flat_map(|field| field.split(','))
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(Self::named)
            .collect()
    }
}

/// Whether `body` opens with a zlib header (RFC 1950): the deflate method
/// and a check over its first two bytes. Chrome reads `deflate` without one
/// as raw deflate.
pub fn has_zlib_header(body: &[u8]) -> bool {
    match body {
        [method, flags, ..] => {
            method & 0x0f == 8 && (u16::from(*method) << 8 | u16::from(*flags)) % 31 == 0
        }
        _ => false,
    }
}

/// One coding's decoder.
enum Decoder {
    Gzip(Box<Gzip>),
    Deflate(Box<Deflate>),
    Brotli(Box<Brotli>),
    Zstd(Box<Zstd>),
}

impl Decoder {
    fn of(coding: ContentCoding) -> Self {
        match coding {
            ContentCoding::Gzip => Self::Gzip(Box::new(Gzip::new())),
            ContentCoding::Deflate => Self::Deflate(Box::new(Deflate::Opening(Vec::new()))),
            ContentCoding::Brotli => Self::Brotli(Box::new(Brotli::new())),
            ContentCoding::Zstd => Self::Zstd(Box::new(Zstd::new())),
        }
    }

    /// Decodes all of `input` into `out`.
    fn push(&mut self, input: &[u8], out: Out<'_>) -> Result<(), Stop> {
        match self {
            Self::Gzip(gzip) => gzip.push(input, out),
            Self::Deflate(deflate) => deflate.push(input, out),
            Self::Brotli(brotli) => brotli.push(input, out),
            Self::Zstd(zstd) => zstd.push(input, out),
        }
    }

    /// Ends the stream once the body ended.
    ///
    /// # Errors
    ///
    /// [`BodyFailure::Cut`] for a stream that stops before its end.
    fn end(&mut self, out: Out<'_>) -> Result<(), Stop> {
        match self {
            Self::Gzip(gzip) => gzip.end(),
            Self::Deflate(deflate) => deflate.end(out),
            Self::Brotli(brotli) => brotli.end(),
            Self::Zstd(zstd) => zstd.end(),
        }
    }
}

/// A deflate stream (RFC 1951), raw or in its zlib wrapper, inflated into
/// `buffer`.
struct Inflate {
    decompress: Decompress,
    buffer: Box<[u8]>,
}

impl Inflate {
    fn new(zlib: bool) -> Self {
        Self {
            decompress: Decompress::new(zlib),
            buffer: vec![0; DECODER_BUFFER].into_boxed_slice(),
        }
    }

    /// Inflates `input` into `out` until its stream ends or the input is
    /// all taken: the bytes it took, and whether the stream ended.
    fn push(&mut self, input: &[u8], out: Out<'_>) -> Result<(usize, bool), Stop> {
        let mut taken = 0;
        loop {
            let (in_before, out_before) = (self.decompress.total_in(), self.decompress.total_out());
            let status = self
                .decompress
                .decompress(&input[taken..], &mut self.buffer, FlushDecompress::None)
                .map_err(|_| BodyFailure::Corrupt)?;
            let consumed = usize::try_from(self.decompress.total_in() - in_before)
                .expect("invariant: a chunk's length fits in usize");
            let produced = usize::try_from(self.decompress.total_out() - out_before)
                .expect("invariant: the buffer's length fits in usize");
            taken += consumed;
            out(&self.buffer[..produced])?;
            if status == Status::StreamEnd {
                return Ok((taken, true));
            }
            let drained = taken == input.len() && produced < self.buffer.len();
            if drained || (consumed == 0 && produced == 0) {
                return Ok((taken, false));
            }
        }
    }
}

/// A `deflate` stream: with its zlib wrapper or raw, as Chrome falls back
/// to raw deflate, which its first two bytes tell.
enum Deflate {
    /// Its first bytes, until there are two.
    Opening(Vec<u8>),
    /// Inflating.
    Inflating(Inflate),
    /// Its stream ended; what follows is ignored.
    Ended,
}

impl Deflate {
    fn push(&mut self, input: &[u8], out: Out<'_>) -> Result<(), Stop> {
        match self {
            Self::Opening(first) => {
                first.extend_from_slice(input);
                if first.len() >= 2 {
                    self.open(out)?;
                }
                Ok(())
            }
            Self::Inflating(inflate) => {
                if inflate.push(input, out)?.1 {
                    *self = Self::Ended;
                }
                Ok(())
            }
            Self::Ended => Ok(()),
        }
    }

    /// Opens the stream on its first bytes, then inflates them.
    fn open(&mut self, out: Out<'_>) -> Result<(), Stop> {
        let Self::Opening(first) = self else {
            return Ok(());
        };
        let first = std::mem::take(first);
        *self = Self::Inflating(Inflate::new(has_zlib_header(&first)));
        self.push(&first, out)
    }

    fn end(&mut self, out: Out<'_>) -> Result<(), Stop> {
        self.open(out)?;
        match self {
            Self::Ended => Ok(()),
            Self::Opening(_) | Self::Inflating(_) => Err(Stop::Failed(BodyFailure::Cut)),
        }
    }
}

/// The gzip header's flags (RFC 1952): its text bit, then the parts that
/// follow its fixed ten bytes, in their order.
const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;
/// The reserved flags, which a stream must not set.
const RESERVED: u8 = 0xe0;

/// A gzip stream (RFC 1952): its header, its deflate stream, then its
/// trailer, the CRC-32 and size of what it decoded, checked. Bytes after
/// it are ignored.
enum Gzip {
    /// Reading its header.
    Header(GzipHeader),
    /// Inflating its body.
    Body(Inflate, Crc),
    /// Reading its eight-byte trailer.
    Trailer(Crc, [u8; 8], usize),
    /// Its stream ended; what follows is ignored.
    Ended,
}

/// A gzip header read so far.
struct GzipHeader {
    /// Its bytes read so far.
    read: usize,
    /// The flags whose parts are still to come.
    flags: u8,
    /// The part being read.
    part: HeaderPart,
}

/// A part of a gzip header.
#[derive(Clone, Copy)]
enum HeaderPart {
    /// Its ten fixed bytes.
    Fixed,
    /// The extra field's length, its first byte read if `Some`.
    ExtraLength(Option<u8>),
    /// The extra field, with this many bytes left.
    Extra(u16),
    /// A zero-terminated name or comment.
    Text,
    /// The header's CRC-16, with this many bytes left.
    Crc(u8),
}

impl GzipHeader {
    /// Reads header bytes from `input`: how many it took, and whether the
    /// header is whole.
    ///
    /// # Errors
    ///
    /// [`BodyFailure::Corrupt`] for bytes no gzip header holds.
    fn read(&mut self, input: &[u8]) -> Result<(usize, bool), BodyFailure> {
        for (at, &byte) in input.iter().enumerate() {
            self.read += 1;
            let done = match self.part {
                HeaderPart::Fixed => self.fixed(byte)?,
                HeaderPart::ExtraLength(None) => {
                    self.part = HeaderPart::ExtraLength(Some(byte));
                    false
                }
                HeaderPart::ExtraLength(Some(low)) => {
                    let length = u16::from_le_bytes([low, byte]);
                    self.part = HeaderPart::Extra(length);
                    length == 0
                }
                HeaderPart::Extra(left) => {
                    self.part = HeaderPart::Extra(left - 1);
                    left == 1
                }
                HeaderPart::Text => byte == 0,
                HeaderPart::Crc(left) => {
                    self.part = HeaderPart::Crc(left - 1);
                    left == 1
                }
            };
            if done && self.next() {
                return Ok((at + 1, true));
            }
        }
        Ok((input.len(), false))
    }

    /// Reads one of the ten fixed bytes: whether they are all in.
    fn fixed(&mut self, byte: u8) -> Result<bool, BodyFailure> {
        let valid = match self.read {
            1 => byte == GZIP_MAGIC[0],
            2 => byte == GZIP_MAGIC[1],
            // Deflate, the one method RFC 1952 defines.
            3 => byte == 8,
            4 => {
                self.flags = byte;
                byte & RESERVED == 0
            }
            _ => true,
        };
        if !valid {
            return Err(BodyFailure::Corrupt);
        }
        Ok(self.read == 10)
    }

    /// Moves to the next part the flags call for, the one read done:
    /// whether the header is whole.
    fn next(&mut self) -> bool {
        let (flag, part) = [
            (FEXTRA, HeaderPart::ExtraLength(None)),
            (FNAME, HeaderPart::Text),
            (FCOMMENT, HeaderPart::Text),
            (FHCRC, HeaderPart::Crc(2)),
        ]
        .into_iter()
        .find(|(flag, _)| self.flags & flag != 0)
        .unwrap_or((0, HeaderPart::Fixed));
        self.flags &= !flag;
        self.part = part;
        flag == 0
    }
}

impl Gzip {
    fn new() -> Self {
        Self::Header(GzipHeader {
            read: 0,
            flags: 0,
            part: HeaderPart::Fixed,
        })
    }

    fn push(&mut self, mut input: &[u8], out: Out<'_>) -> Result<(), Stop> {
        while !input.is_empty() {
            match self {
                Self::Header(header) => {
                    let (taken, whole) = header.read(input)?;
                    input = &input[taken..];
                    if whole {
                        *self = Self::Body(Inflate::new(false), Crc::new());
                    }
                }
                Self::Body(inflate, crc) => {
                    let (taken, ended) = inflate.push(input, &mut |decoded| {
                        crc.update(decoded);
                        out(decoded)
                    })?;
                    input = &input[taken..];
                    if ended {
                        *self = Self::Trailer(std::mem::replace(crc, Crc::new()), [0; 8], 0);
                    }
                }
                Self::Trailer(crc, trailer, read) => {
                    let taken = (trailer.len() - *read).min(input.len());
                    trailer[*read..*read + taken].copy_from_slice(&input[..taken]);
                    *read += taken;
                    input = &input[taken..];
                    if *read == trailer.len() {
                        let [c0, c1, c2, c3, s0, s1, s2, s3] = *trailer;
                        let checked = u32::from_le_bytes([c0, c1, c2, c3]) == crc.sum()
                            && u32::from_le_bytes([s0, s1, s2, s3]) == crc.amount();
                        if !checked {
                            return Err(Stop::Failed(BodyFailure::Corrupt));
                        }
                        *self = Self::Ended;
                    }
                }
                Self::Ended => return Ok(()),
            }
        }
        Ok(())
    }

    fn end(&self) -> Result<(), Stop> {
        match self {
            Self::Ended => Ok(()),
            // Too few bytes to open as a gzip stream: no gzip stream.
            Self::Header(header) if header.read < GZIP_MAGIC.len() => {
                Err(Stop::Failed(BodyFailure::Corrupt))
            }
            Self::Header(_) | Self::Body(..) | Self::Trailer(..) => {
                Err(Stop::Failed(BodyFailure::Cut))
            }
        }
    }
}

/// A Brotli stream (RFC 7932), through the decoder's stream interface,
/// which tells a stream that wants more input, a cut one, from an invalid
/// one.
struct Brotli {
    state: brotli_decompressor::BrotliState<
        brotli_decompressor::StandardAlloc,
        brotli_decompressor::StandardAlloc,
        brotli_decompressor::StandardAlloc,
    >,
    buffer: Box<[u8]>,
    /// The bytes it wrote out so far, which the decoder counts.
    written: usize,
    /// Whether its stream ended.
    ended: bool,
}

impl Brotli {
    fn new() -> Self {
        use brotli_decompressor::StandardAlloc;
        Self {
            state: brotli_decompressor::BrotliState::new(
                StandardAlloc::default(),
                StandardAlloc::default(),
                StandardAlloc::default(),
            ),
            buffer: vec![0; DECODER_BUFFER].into_boxed_slice(),
            written: 0,
            ended: false,
        }
    }

    fn push(&mut self, input: &[u8], out: Out<'_>) -> Result<(), Stop> {
        use brotli_decompressor::{BrotliDecompressStream, BrotliResult};

        if input.is_empty() {
            return Ok(());
        }
        // Bytes after the stream's end do not decode.
        if self.ended {
            return Err(Stop::Failed(BodyFailure::Corrupt));
        }
        let mut available_in = input.len();
        let mut input_offset = 0;
        loop {
            let mut available_out = self.buffer.len();
            let mut output_offset = 0;
            let result = BrotliDecompressStream(
                &mut available_in,
                &mut input_offset,
                input,
                &mut available_out,
                &mut output_offset,
                &mut self.buffer,
                &mut self.written,
                &mut self.state,
            );
            out(&self.buffer[..output_offset])?;
            match result {
                BrotliResult::NeedsMoreOutput => {}
                BrotliResult::NeedsMoreInput => return Ok(()),
                BrotliResult::ResultSuccess if available_in == 0 => {
                    self.ended = true;
                    return Ok(());
                }
                BrotliResult::ResultSuccess | BrotliResult::ResultFailure => {
                    return Err(Stop::Failed(BodyFailure::Corrupt));
                }
            }
        }
    }

    fn end(&self) -> Result<(), Stop> {
        if self.ended {
            Ok(())
        } else {
            Err(Stop::Failed(BodyFailure::Cut))
        }
    }
}

/// A zstd stream (RFC 8878), whose frames may ask for a window of at most
/// 8 MiB.
struct Zstd {
    /// The decoder; `None` when none could start, which no body decodes.
    decoder: Option<zstd::stream::raw::Decoder<'static>>,
    buffer: Box<[u8]>,
    /// Whether the last frame read ended and gave all it decodes; none
    /// read yet is a cut stream.
    whole: bool,
}

impl Zstd {
    fn new() -> Self {
        let decoder = zstd::stream::raw::Decoder::new()
            .ok()
            .and_then(|mut decoder| {
                decoder
                    .set_parameter(zstd::stream::raw::DParameter::WindowLogMax(
                        ZSTD_WINDOW_LOG_MAX,
                    ))
                    .ok()
                    .map(|()| decoder)
            });
        Self {
            decoder,
            buffer: vec![0; DECODER_BUFFER].into_boxed_slice(),
            whole: false,
        }
    }

    fn push(&mut self, input: &[u8], out: Out<'_>) -> Result<(), Stop> {
        use zstd::stream::raw::{InBuffer, Operation as _, OutBuffer};

        let decoder = self
            .decoder
            .as_mut()
            .ok_or(Stop::Failed(BodyFailure::Corrupt))?;
        let mut input = InBuffer::around(input);
        loop {
            let mut output = OutBuffer::around(&mut self.buffer[..]);
            let before = input.pos();
            // A frame past the window, as any other error, does not decode.
            let hint = decoder
                .run(&mut input, &mut output)
                .map_err(|_| BodyFailure::Corrupt)?;
            let produced = output.pos();
            // A frame that ended stays whole until a byte of another is
            // read: a call past its end asks for the next frame's header.
            if hint == 0 {
                self.whole = true;
            } else if input.pos() > before {
                self.whole = false;
            }
            out(&self.buffer[..produced])?;
            if input.pos() == input.src.len() && produced < self.buffer.len() {
                return Ok(());
            }
        }
    }

    fn end(&self) -> Result<(), Stop> {
        match (&self.decoder, self.whole) {
            (None, _) => Err(Stop::Failed(BodyFailure::Corrupt)),
            (Some(_), true) => Ok(()),
            (Some(_), false) => Err(Stop::Failed(BodyFailure::Cut)),
        }
    }
}
