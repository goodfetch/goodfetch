//! An Export's pipeline (ADRs 0103 and 0119): a table format's column pass
//! over its Records, then their encoding, compressed, a page at a time,
//! each committed Record as the encoder takes it, its Wall marker
//! included. `run.export` drives it over the Run reader, and
//! [`write_export`] over a run file's connection, for the Compatibility
//! corpus and the benchmarks, so both write the bytes a user's Export holds.

use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use rusqlite::Connection;

use crate::encoding::{Columns, Compressor, ExportEncoder, ExportOptions, Exported};
use crate::records::{CommittedText, Pages, last_number};

/// One Export's pipeline before it encodes: its format, its options, its
/// compression and, for a table, the columns seen so far.
#[derive(Debug)]
pub(crate) struct Pipeline {
    format: ExportFormat,
    options: ExportOptions,
    compression: ExportCompression,
    columns: Columns,
}

impl Pipeline {
    /// The pipeline of an Export in `format` with `options`, compressed as
    /// `compression` says, which has seen no Record yet.
    pub(crate) fn new(
        format: ExportFormat,
        options: ExportOptions,
        compression: ExportCompression,
    ) -> Self {
        let columns = Columns::new(options.nested(format));
        Self {
            format,
            options,
            compression,
            columns,
        }
    }

    /// Whether it reads the Records once for their columns before it
    /// encodes them: a table format does (pitfall D07).
    pub(crate) fn reads_columns(&self) -> bool {
        self.format.is_table()
    }

    /// Sees the fields of `page`'s Records, each with its Wall marker, for
    /// a table's columns.
    pub(crate) fn see(&mut self, page: &[CommittedText]) {
        for record in page {
            self.columns.see(&exported(record).marked());
        }
    }

    /// Its encoding, with the Export's first bytes written.
    pub(crate) fn encoding(self) -> Encoding {
        let mut encoder = ExportEncoder::new(self.format, &self.options, self.columns);
        let mut bytes = Vec::new();
        encoder.begin(&mut bytes);
        Encoding {
            encoder,
            compressor: Compressor::new(self.compression),
            bytes,
        }
    }
}

/// One Export's encoding, Record by Record, into compressed bytes.
#[derive(Debug)]
pub(crate) struct Encoding {
    encoder: ExportEncoder,
    compressor: Compressor,
    bytes: Vec<u8>,
}

impl Encoding {
    /// Encodes `page`'s Records, after those before.
    pub(crate) fn write(&mut self, page: &[CommittedText]) {
        for record in page {
            self.encoder.value(&exported(record), &mut self.bytes);
        }
    }

    /// The compressed bytes of what it encoded since it last answered them.
    pub(crate) fn take(&mut self) -> Vec<u8> {
        self.compressor.write(std::mem::take(&mut self.bytes))
    }

    /// How many Records it encoded.
    pub(crate) fn records(&self) -> u64 {
        self.encoder.values()
    }

    /// Writes the Export's end, and answers the file's last bytes: what it
    /// encoded since it last answered, the end, and the compression's end.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.encoder.end(&mut self.bytes);
        let mut compressed = self.compressor.write(self.bytes);
        compressed.append(&mut self.compressor.finish());
        compressed
    }
}

/// A committed Record as the encoder takes it.
fn exported(record: &CommittedText) -> Exported<'_> {
    Exported {
        number: record.number,
        value: &record.value,
        wall: record.wall.as_deref(),
        committed_at: &record.committed_at,
    }
}

/// The Export of the Dataset `dataset`'s Records in the run file
/// `connection`, through the last one committed when it begins, in
/// `format` with `options`, compressed as `compression` says: the file
/// `run.export` writes of them, read and encoded the same way, a page at a
/// time.
///
/// # Errors
///
/// The run file's, when it cannot be read.
pub fn write_export(
    connection: &Connection,
    dataset: &str,
    format: ExportFormat,
    options: ExportOptions,
    compression: ExportCompression,
) -> rusqlite::Result<Vec<u8>> {
    let through = Some(last_number(connection, dataset)?);
    let mut pipeline = Pipeline::new(format, options, compression);
    if pipeline.reads_columns() {
        let mut pages = Pages::new(dataset, 0, through, None);
        loop {
            let (page, full) = pages.values(connection)?;
            pipeline.see(&page);
            if !full {
                break;
            }
        }
    }
    let mut encoding = pipeline.encoding();
    let mut pages = Pages::new(dataset, 0, through, None);
    let mut file = Vec::new();
    loop {
        let (page, full) = pages.values(connection)?;
        encoding.write(&page);
        if !full {
            break;
        }
        file.append(&mut encoding.take());
    }
    file.append(&mut encoding.finish());
    Ok(file)
}
