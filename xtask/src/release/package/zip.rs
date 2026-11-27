//! Minimal deterministic ZIP archive writer for `py3-none` Python wheels,
//! built on `flate2` so `xtask` adds no ZIP crate dependency and preserves
//! Unix executable permissions (`0o100755`) on the bundled engine binary.

use std::io::Write as _;

use anyhow::{Result, ensure};
use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc};

const LOCAL_FILE_HEADER_SIG: u32 = 0x0403_4b50;
const CENTRAL_DIR_HEADER_SIG: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIR_SIG: u32 = 0x0605_4b50;
const VERSION_MADE_BY_UNIX: u16 = (3 << 8) | 0x0014;
const VERSION_NEEDED_DEFLATE: u16 = 20;
const METHOD_DEFLATE: u16 = 8;
/// Fixed DOS timestamp: 1980-01-01 00:00:00 for reproducible wheels.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

struct EntryRecord {
    name: Vec<u8>,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
    unix_mode: u32,
}

/// Builds a deterministic ZIP archive in memory.
pub(super) struct ZipWriter {
    buffer: Vec<u8>,
    entries: Vec<EntryRecord>,
}

impl ZipWriter {
    pub(super) fn new() -> Self {
        Self {
            buffer: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Adds a regular file with Unix permission bits `mode` (such as `0o644`
    /// or `0o755`), deflated with deterministic timestamps.
    pub(super) fn add_file(&mut self, name: &str, data: &[u8], mode: u32) -> Result<()> {
        let name_bytes = name.as_bytes();
        let name_len = u16::try_from(name_bytes.len())?;
        let uncompressed_size = u32::try_from(data.len())?;
        let local_header_offset = u32::try_from(self.buffer.len())?;

        let mut crc = Crc::new();
        crc.update(data);
        let crc32 = crc.sum();

        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data)?;
        let compressed = encoder.finish()?;
        let compressed_size = u32::try_from(compressed.len())?;

        self.buffer
            .extend_from_slice(&LOCAL_FILE_HEADER_SIG.to_le_bytes());
        self.buffer
            .extend_from_slice(&VERSION_NEEDED_DEFLATE.to_le_bytes());
        self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // flags
        self.buffer.extend_from_slice(&METHOD_DEFLATE.to_le_bytes());
        self.buffer.extend_from_slice(&DOS_TIME.to_le_bytes());
        self.buffer.extend_from_slice(&DOS_DATE.to_le_bytes());
        self.buffer.extend_from_slice(&crc32.to_le_bytes());
        self.buffer
            .extend_from_slice(&compressed_size.to_le_bytes());
        self.buffer
            .extend_from_slice(&uncompressed_size.to_le_bytes());
        self.buffer.extend_from_slice(&name_len.to_le_bytes());
        self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // extra len
        self.buffer.extend_from_slice(name_bytes);
        self.buffer.extend_from_slice(&compressed);

        self.entries.push(EntryRecord {
            name: name_bytes.to_vec(),
            crc32,
            compressed_size,
            uncompressed_size,
            local_header_offset,
            unix_mode: 0o100_000 | (mode & 0o777),
        });
        Ok(())
    }

    /// Writes the central directory and end-of-central-directory record,
    /// returning the complete ZIP bytes.
    pub(super) fn finish(mut self) -> Result<Vec<u8>> {
        let central_offset = u32::try_from(self.buffer.len())?;
        let count = u16::try_from(self.entries.len())?;

        for entry in &self.entries {
            let name_len = u16::try_from(entry.name.len())?;
            let external_attrs = entry.unix_mode << 16;

            self.buffer
                .extend_from_slice(&CENTRAL_DIR_HEADER_SIG.to_le_bytes());
            self.buffer
                .extend_from_slice(&VERSION_MADE_BY_UNIX.to_le_bytes());
            self.buffer
                .extend_from_slice(&VERSION_NEEDED_DEFLATE.to_le_bytes());
            self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // flags
            self.buffer.extend_from_slice(&METHOD_DEFLATE.to_le_bytes());
            self.buffer.extend_from_slice(&DOS_TIME.to_le_bytes());
            self.buffer.extend_from_slice(&DOS_DATE.to_le_bytes());
            self.buffer.extend_from_slice(&entry.crc32.to_le_bytes());
            self.buffer
                .extend_from_slice(&entry.compressed_size.to_le_bytes());
            self.buffer
                .extend_from_slice(&entry.uncompressed_size.to_le_bytes());
            self.buffer.extend_from_slice(&name_len.to_le_bytes());
            self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // extra len
            self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // comment len
            self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // disk start
            self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // internal attrs
            self.buffer.extend_from_slice(&external_attrs.to_le_bytes());
            self.buffer
                .extend_from_slice(&entry.local_header_offset.to_le_bytes());
            self.buffer.extend_from_slice(&entry.name);
        }

        let central_size = u32::try_from(self.buffer.len() - usize::try_from(central_offset)?)?;
        ensure!(
            self.entries.len() == usize::from(count),
            "too many entries for ZIP"
        );

        self.buffer
            .extend_from_slice(&END_OF_CENTRAL_DIR_SIG.to_le_bytes());
        self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // disk number
        self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // central dir disk
        self.buffer.extend_from_slice(&count.to_le_bytes());
        self.buffer.extend_from_slice(&count.to_le_bytes());
        self.buffer.extend_from_slice(&central_size.to_le_bytes());
        self.buffer.extend_from_slice(&central_offset.to_le_bytes());
        self.buffer.extend_from_slice(&0_u16.to_le_bytes()); // comment len
        Ok(self.buffer)
    }
}
