//! An Export's pipeline (ADRs 0103 and 0119): a table format's column pass
//! over its Records, then their encoding, compressed, a page at a time,
//! each committed Record as the encoder takes it, its Wall marker
//! included. `run.export` drives it over the Run reader, and
//! [`write_export`] over a run file's connection, for the Compatibility
//! corpus and the benchmarks, so both write the bytes a user's Export holds.

use goodfetch_engine::CallError;
use goodfetch_protocol::foxhound::export::{
    ExportCompression, ExportFormat, SHEET_CAPACITY_EXCEEDED, SheetCapacityExceededDetails,
};
use rusqlite::Connection;

use crate::encoding::{
    Columns, Compressor, ExportEncoder, ExportOptions, Exported, SheetCapacityExceeded, sheet_fits,
};
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

    /// Its encoding of the Dataset `dataset`, with the Export's first bytes
    /// written.
    ///
    /// # Errors
    ///
    /// `sheet_capacity_exceeded`, before any byte is written, for an XLSX
    /// Export whose sheet would pass what Excel opens.
    pub(crate) fn encoding(self, dataset: &str) -> Result<Encoding, CallError> {
        if self.format == ExportFormat::Xlsx {
            sheet_fits(&self.columns).map_err(|refused| sheet_refused(dataset, refused))?;
        }
        let mut encoder = ExportEncoder::new(self.format, &self.options, self.columns);
        let mut compressor = Compressor::new(self.compression);
        encoder.begin(compressor.input());
        Ok(Encoding {
            encoder,
            compressor,
        })
    }
}

/// One Export's encoding, Record by Record, into compressed bytes.
#[derive(Debug)]
pub(crate) struct Encoding {
    encoder: ExportEncoder,
    compressor: Compressor,
}

impl Encoding {
    /// Encodes `page`'s Records, after those before.
    pub(crate) fn write(&mut self, page: &[CommittedText]) {
        let out = self.compressor.input();
        for record in page {
            self.encoder.value(&exported(record), out);
        }
    }

    /// Compresses what it encoded since it last did, after the file's
    /// bytes so far.
    pub(crate) fn compress(&mut self) {
        self.compressor.compress();
    }

    /// The compressed bytes of what it encoded since it last answered them.
    pub(crate) fn take(&mut self) -> Vec<u8> {
        self.compressor.take()
    }

    /// How many Records it encoded.
    pub(crate) fn records(&self) -> u64 {
        self.encoder.values()
    }

    /// Writes the Export's end, and answers the file's last bytes: what it
    /// encoded since it last answered, the end, and the compression's end.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.encoder.end(self.compressor.input());
        self.compressor.finish()
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

/// `sheet_capacity_exceeded` for the XLSX Export of `dataset` that
/// `refused` names, of which nothing was written.
fn sheet_refused(dataset: &str, refused: SheetCapacityExceeded) -> CallError {
    let fix = format!(
        "Export `{dataset}` as `csv`, `jsonl` or `parquet`, which hold any number of rows, columns and characters."
    );
    CallError::new(
        &SHEET_CAPACITY_EXCEEDED,
        format!("The XLSX Export of `{dataset}` was refused: {refused}. {fix}"),
        &SheetCapacityExceededDetails {
            dataset: dataset.to_owned(),
            capacity: refused.capacity,
            figure: refused.figure,
            maximum: refused.capacity.maximum(),
            fix,
        },
    )
}

/// Why [`write_export`] wrote no Export.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WriteExportError {
    /// The run file could not be read.
    #[error("the run file could not be read: {0}")]
    Read(#[from] rusqlite::Error),
    /// The Export was refused, as `run.export` refuses it:
    /// `sheet_capacity_exceeded` for an XLSX sheet past what Excel opens.
    #[error(transparent)]
    Refused(#[from] CallError),
}

/// The Export of the Dataset `dataset`'s Records in the run file
/// `connection`, through the last one committed when it begins, in
/// `format` with `options`, compressed as `compression` says: the file
/// `run.export` writes of them, read and encoded the same way, a page at a
/// time.
///
/// # Errors
///
/// The run file's, when it cannot be read, and an XLSX Export's whose
/// sheet would pass what Excel opens.
pub fn write_export(
    connection: &Connection,
    dataset: &str,
    format: ExportFormat,
    options: ExportOptions,
    compression: ExportCompression,
) -> Result<Vec<u8>, WriteExportError> {
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
    let mut encoding = pipeline.encoding(dataset)?;
    let mut pages = Pages::new(dataset, 0, through, None);
    loop {
        let (page, full) = pages.values(connection)?;
        encoding.write(&page);
        if !full {
            break;
        }
        encoding.compress();
    }
    Ok(encoding.finish())
}

#[cfg(test)]
mod tests;
