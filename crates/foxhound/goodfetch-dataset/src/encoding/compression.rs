//! An Export's compression, over the whole file as its format writes it:
//! one gzip member (RFC 1952) at flate2's default level, with no file name
//! and a zero modification time, or one Zstandard frame (RFC 8878) at
//! zstd's default level; either way the same bytes give the same file.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_protocol::foxhound::export::ExportCompression;

/// The compressor of one Export's bytes, as they are written.
pub(crate) struct Compressor(Inner);

enum Inner {
    None,
    Gzip(GzEncoder<Vec<u8>>),
    Zstd(zstd::stream::write::Encoder<'static, Vec<u8>>),
}

impl std::fmt::Debug for Compressor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.0 {
            Inner::None => "Compressor(None)",
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
            ExportCompression::None => Inner::None,
            ExportCompression::Gzip => {
                Inner::Gzip(GzEncoder::new(Vec::new(), Compression::default()))
            }
            ExportCompression::Zstd => Inner::Zstd(
                zstd::stream::write::Encoder::new(Vec::new(), 0)
                    .expect("invariant: a zstd encoder starts in memory"),
            ),
        })
    }

    /// Compresses `bytes`, the file's next, and answers what it can hand
    /// on so far.
    #[must_use]
    pub(crate) fn write(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        match &mut self.0 {
            Inner::None => bytes,
            Inner::Gzip(encoder) => {
                encoder
                    .write_all(&bytes)
                    .expect("invariant: compressing into memory never fails");
                std::mem::take(encoder.get_mut())
            }
            Inner::Zstd(encoder) => {
                encoder
                    .write_all(&bytes)
                    .expect("invariant: compressing into memory never fails");
                std::mem::take(encoder.get_mut())
            }
        }
    }

    /// The compressed file's last bytes, once every byte was written.
    #[must_use]
    pub(crate) fn finish(self) -> Vec<u8> {
        match self.0 {
            Inner::None => Vec::new(),
            Inner::Gzip(encoder) => encoder
                .finish()
                .expect("invariant: compressing into memory never fails"),
            Inner::Zstd(encoder) => encoder
                .finish()
                .expect("invariant: compressing into memory never fails"),
        }
    }
}
