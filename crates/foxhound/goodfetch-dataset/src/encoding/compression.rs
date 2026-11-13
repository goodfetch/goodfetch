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
    /// No compression: the file's bytes are the bytes written.
    None(Vec<u8>),
    Gzip(GzEncoder<Vec<u8>>),
    Zstd(zstd::stream::write::Encoder<'static, Vec<u8>>),
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
            ExportCompression::Gzip => {
                Inner::Gzip(GzEncoder::new(Vec::new(), Compression::default()))
            }
            ExportCompression::Zstd => Inner::Zstd(
                zstd::stream::write::Encoder::new(Vec::new(), 0)
                    .expect("invariant: a zstd encoder starts in memory"),
            ),
        })
    }

    /// The file's bytes not yet taken, when it compresses nothing: an
    /// encoder writes the file's next bytes straight into them.
    pub(crate) fn plain(&mut self) -> Option<&mut Vec<u8>> {
        match &mut self.0 {
            Inner::None(file) => Some(file),
            Inner::Gzip(_) | Inner::Zstd(_) => None,
        }
    }

    /// Compresses `bytes`, the file's next.
    pub(crate) fn write(&mut self, bytes: &[u8]) {
        match &mut self.0 {
            Inner::None(file) => file.extend_from_slice(bytes),
            Inner::Gzip(encoder) => encoder
                .write_all(bytes)
                .expect("invariant: compressing into memory never fails"),
            Inner::Zstd(encoder) => encoder
                .write_all(bytes)
                .expect("invariant: compressing into memory never fails"),
        }
    }

    /// The file's bytes it made since they were last taken, which it can
    /// hand on so far; the next start in a buffer of the same capacity.
    #[must_use]
    pub(crate) fn take(&mut self) -> Vec<u8> {
        let file = match &mut self.0 {
            Inner::None(file) => file,
            Inner::Gzip(encoder) => encoder.get_mut(),
            Inner::Zstd(encoder) => encoder.get_mut(),
        };
        let capacity = file.capacity();
        std::mem::replace(file, Vec::with_capacity(capacity))
    }

    /// The compressed file's last bytes, once every byte was written: those
    /// not yet taken and the compression's end.
    #[must_use]
    pub(crate) fn finish(self) -> Vec<u8> {
        match self.0 {
            Inner::None(file) => file,
            Inner::Gzip(encoder) => encoder
                .finish()
                .expect("invariant: compressing into memory never fails"),
            Inner::Zstd(encoder) => encoder
                .finish()
                .expect("invariant: compressing into memory never fails"),
        }
    }
}
