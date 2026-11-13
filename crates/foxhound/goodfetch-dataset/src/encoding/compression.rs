//! An Export's compression, over the whole file as its format writes it:
//! one gzip member (RFC 1952) at flate2's default level, with no file name
//! and a zero modification time, or one Zstandard frame (RFC 8878) at
//! zstd's default level; either way the same bytes give the same file.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_protocol::foxhound::export::ExportCompression;

/// The compressor of one Export's bytes, as they are written, holding the
/// file's bytes it made of them until they are taken.
pub(crate) struct Compressor(Inner);

enum Inner {
    /// No compression: the file's bytes are the bytes written, straight
    /// into them.
    None(Vec<u8>),
    Gzip(Staged<GzEncoder<Vec<u8>>>),
    Zstd(Staged<zstd::stream::write::Encoder<'static, Vec<u8>>>),
}

/// A compression's encoder, which makes the file's bytes, and the bytes
/// written since it last compressed, whose capacity is kept from one part
/// to the next.
struct Staged<E> {
    encoder: E,
    written: Vec<u8>,
}

impl<E: Write> Staged<E> {
    fn new(encoder: E) -> Self {
        Self {
            encoder,
            written: Vec::new(),
        }
    }

    fn compress(&mut self) {
        self.encoder
            .write_all(&self.written)
            .expect("invariant: compressing into memory never fails");
        self.written.clear();
    }
}

impl std::fmt::Debug for Compressor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.0 {
            Inner::None(_) => "Compressor(None)",
            Inner::Gzip(_) => "Compressor(Gzip)",
            Inner::Zstd(_) => "Compressor(Zstd)",
        })
    }
}

impl Compressor {
    /// The compressor `compression` names.
    #[must_use]
    pub(crate) fn new(compression: ExportCompression) -> Self {
        Self(match compression {
            ExportCompression::None => Inner::None(Vec::new()),
            ExportCompression::Gzip => Inner::Gzip(Staged::new(GzEncoder::new(
                Vec::new(),
                Compression::default(),
            ))),
            ExportCompression::Zstd => Inner::Zstd(Staged::new(
                zstd::stream::write::Encoder::new(Vec::new(), 0)
                    .expect("invariant: a zstd encoder starts in memory"),
            )),
        })
    }

    /// Where the file's next bytes are written: the file's own bytes when
    /// it compresses nothing, else bytes [`Compressor::compress`] compresses.
    pub(crate) fn input(&mut self) -> &mut Vec<u8> {
        match &mut self.0 {
            Inner::None(file) => file,
            Inner::Gzip(staged) => &mut staged.written,
            Inner::Zstd(staged) => &mut staged.written,
        }
    }

    /// Compresses the bytes written since it last did, in one write.
    pub(crate) fn compress(&mut self) {
        match &mut self.0 {
            Inner::None(_) => {}
            Inner::Gzip(staged) => staged.compress(),
            Inner::Zstd(staged) => staged.compress(),
        }
    }

    /// Hands over the file's bytes made since they were last taken, the
    /// bytes written being compressed first. The bytes made next go into a
    /// new buffer with the capacity these reached.
    #[must_use]
    pub(crate) fn take(&mut self) -> Vec<u8> {
        self.compress();
        let file = match &mut self.0 {
            Inner::None(file) => file,
            Inner::Gzip(staged) => staged.encoder.get_mut(),
            Inner::Zstd(staged) => staged.encoder.get_mut(),
        };
        let capacity = file.capacity();
        std::mem::replace(file, Vec::with_capacity(capacity))
    }

    /// The file's last bytes, once every byte was written: those not yet
    /// taken, and the compression's end.
    #[must_use]
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.compress();
        match self.0 {
            Inner::None(file) => file,
            Inner::Gzip(staged) => staged
                .encoder
                .finish()
                .expect("invariant: compressing into memory never fails"),
            Inner::Zstd(staged) => staged
                .encoder
                .finish()
                .expect("invariant: compressing into memory never fails"),
        }
    }
}
