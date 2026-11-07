//! Arrow IPC and Parquet: one file of one nullable column per field seen,
//! typed by every value seen in it: `Boolean`, `Int64`, `Float64`, or
//! `Utf8` holding each cell's text. Rows are gathered into batches of at
//! most [`BATCH`], each written as one Arrow record batch, or one Parquet
//! row group, compressed as `export.parquet.compression` says. Both
//! writers own their output, so the file's bytes are taken from a shared
//! buffer after each batch.

use std::sync::Arc;

use arrow_array::builder::{BooleanBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch, RecordBatchOptions};
use arrow_ipc::writer::FileWriter;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

use super::columns::{Cell, Kind};
use super::written::Written;

/// The most rows one batch holds.
const BATCH: usize = 8192;

/// A Parquet file's codec, `export.parquet.compression`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParquetCodec {
    /// Zstandard.
    Zstd,
    /// Snappy.
    Snappy,
    /// None.
    None,
}

impl ParquetCodec {
    /// The codec `choice`, one of `PARQUET_COMPRESSION_VALUES`, names.
    pub fn of(choice: &str) -> Option<Self> {
        match choice {
            "zstd" => Some(Self::Zstd),
            "snappy" => Some(Self::Snappy),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// Which file a columnar writer writes.
enum Sink {
    Arrow(FileWriter<Written>),
    Parquet(ArrowWriter<Written>),
}

/// The writer of one Arrow IPC or Parquet Export.
pub(super) struct Columnar {
    schema: SchemaRef,
    kinds: Vec<Kind>,
    written: Written,
    sink: Option<Sink>,
    rows: Vec<Vec<Cell>>,
}

impl std::fmt::Debug for Columnar {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Columnar")
            .field("schema", &self.schema)
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl Columnar {
    /// An Arrow IPC writer, or a Parquet one with `parquet`'s codec.
    pub(super) fn new(names: &[String], kinds: &[Kind], parquet: Option<ParquetCodec>) -> Self {
        let schema: SchemaRef = Arc::new(Schema::new(
            names
                .iter()
                .zip(kinds)
                .map(|(name, kind)| Field::new(name, data_type(*kind), true))
                .collect::<Vec<_>>(),
        ));
        let written = Written::default();
        let sink = match parquet {
            None => Sink::Arrow(
                FileWriter::try_new(written.clone(), &schema)
                    .expect("invariant: an Arrow file starts in memory"),
            ),
            Some(codec) => {
                let compression = match codec {
                    ParquetCodec::Zstd => Compression::ZSTD(ZstdLevel::default()),
                    ParquetCodec::Snappy => Compression::SNAPPY,
                    ParquetCodec::None => Compression::UNCOMPRESSED,
                };
                let properties = WriterProperties::builder()
                    .set_compression(compression)
                    .build();
                Sink::Parquet(
                    ArrowWriter::try_new(written.clone(), Arc::clone(&schema), Some(properties))
                        .expect("invariant: a Parquet file starts in memory"),
                )
            }
        };
        Self {
            schema,
            kinds: kinds.to_vec(),
            written,
            sink: Some(sink),
            rows: Vec::new(),
        }
    }

    pub(super) fn begin(&mut self, out: &mut Vec<u8>) {
        self.written.take(out);
    }

    pub(super) fn row(&mut self, cells: Vec<Cell>, out: &mut Vec<u8>) {
        self.rows.push(cells);
        if self.rows.len() == BATCH {
            self.flush(out);
        }
    }

    pub(super) fn end(&mut self, out: &mut Vec<u8>) {
        self.flush(out);
        match self
            .sink
            .take()
            .expect("invariant: a columnar Export ends once")
        {
            Sink::Arrow(mut writer) => writer
                .finish()
                .expect("invariant: an Arrow file finishes in memory"),
            Sink::Parquet(writer) => {
                writer
                    .close()
                    .expect("invariant: a Parquet file finishes in memory");
            }
        }
        self.written.take(out);
    }

    /// Writes the rows gathered as one batch.
    fn flush(&mut self, out: &mut Vec<u8>) {
        if self.rows.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.rows);
        let columns: Vec<ArrayRef> = self
            .kinds
            .iter()
            .enumerate()
            .map(|(column, kind)| array(*kind, rows.iter().map(|row| &row[column])))
            .collect();
        let batch = RecordBatch::try_new_with_options(
            Arc::clone(&self.schema),
            columns,
            &RecordBatchOptions::new().with_row_count(Some(rows.len())),
        )
        .expect("invariant: each column holds one cell per row of its type");
        match self
            .sink
            .as_mut()
            .expect("invariant: a columnar Export's rows come before its end")
        {
            Sink::Arrow(writer) => writer
                .write(&batch)
                .expect("invariant: an Arrow batch writes in memory"),
            Sink::Parquet(writer) => writer
                .write(&batch)
                .and_then(|()| writer.flush())
                .expect("invariant: a Parquet row group writes in memory"),
        }
        self.written.take(out);
    }
}

fn data_type(kind: Kind) -> DataType {
    match kind {
        Kind::Boolean => DataType::Boolean,
        Kind::Integer => DataType::Int64,
        Kind::Float => DataType::Float64,
        Kind::Unseen | Kind::Text => DataType::Utf8,
    }
}

/// One column of `cells`, of `kind`.
fn array<'a>(kind: Kind, cells: impl Iterator<Item = &'a Cell>) -> ArrayRef {
    match kind {
        Kind::Boolean => {
            let mut builder = BooleanBuilder::new();
            for cell in cells {
                builder.append_option(match cell {
                    Cell::Boolean(boolean) => Some(*boolean),
                    _ => None,
                });
            }
            Arc::new(builder.finish())
        }
        Kind::Integer => {
            let mut builder = Int64Builder::new();
            for cell in cells {
                builder.append_option(match cell {
                    Cell::Number(text) => Some(
                        text.parse()
                            .expect("invariant: an integer column holds 64-bit integers"),
                    ),
                    _ => None,
                });
            }
            Arc::new(builder.finish())
        }
        Kind::Float => {
            let mut builder = Float64Builder::new();
            for cell in cells {
                builder.append_option(match cell {
                    Cell::Number(text) => Some(
                        text.parse()
                            .expect("invariant: a float column holds numbers"),
                    ),
                    _ => None,
                });
            }
            Arc::new(builder.finish())
        }
        Kind::Unseen | Kind::Text => {
            let mut builder = StringBuilder::new();
            for cell in cells {
                match cell {
                    Cell::Empty => builder.append_null(),
                    other => builder.append_value(other.text()),
                }
            }
            Arc::new(builder.finish())
        }
    }
}
