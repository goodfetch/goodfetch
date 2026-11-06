//! A Dataset's Export (ADRs 0103, 0116, 0119 and 0122): `run.export` reads
//! the number of the Dataset's last committed Record when the call is
//! made, then streams the Records up to it, in number order, a page at a
//! time on the Run reader's read-only connection, through the encoder into
//! the engine's keeping of an Export, which writes them whole into the
//! Run's folder or not at all; so it serves a live Run and one that ended,
//! and Records committed meanwhile are not in it. A table format reads the
//! same Records twice: once for the columns, the fields every Record holds
//! in first-seen order, then for the rows. Its `export.*` settings resolve
//! in the call's own scope: the machine's layers, the call's project
//! configuration and its run layer. `export.save_as` and `export.stream`
//! read that file back through the engine's saving and sending of a kept
//! file.

use std::io::{self, Read};

use goodfetch_engine::run::{run_error, settings_error};
use goodfetch_engine::run_files::export_file;
use goodfetch_engine::run_files_operations::{keep_error, save_kept, send_kept};
use goodfetch_engine::runs::{RunError, RunReader};
use goodfetch_engine::settings::{Opening, Resolved};
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::foxhound::export::{
    EXPORT_CSV_BOM, EXPORT_CSV_DELIMITER, EXPORT_CSV_NESTED, EXPORT_FEED_LINK,
    EXPORT_FEED_PUBLISHED, EXPORT_FEED_SUMMARY, EXPORT_FEED_TITLE, EXPORT_JSON_INDENT,
    EXPORT_PARQUET_COMPRESSION, EXPORT_SAVE_AS, EXPORT_STREAM, EXPORT_XLSX_SHEET,
    EXPORT_XML_RECORD, EXPORT_XML_ROOT, Export, ExportFormat, ExportId, ExportSaveAsParams,
    ExportSaved, ExportStreamParams, ExportStreamed, RUN_EXPORT, RunExportParams,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{
    RunOrigin, SETTING_INVALID, SettingInvalidDetails, SettingsLayer,
};
use tokio::sync::mpsc;

use crate::declaration::Datasets;
use crate::encoding::{
    Columns, Compressor, ExportEncoder, ExportOptions, Exported, Nested, ParquetCodec,
    csv_delimiter, feed_field, json_indent, sheet_name, xml_name,
};
use crate::records::{self, CommittedText, Query};

/// The most Records one read of the run file takes.
const PAGE: usize = 256;

/// How many encoded pages wait for the file's writer at most.
const QUEUED: usize = 4;

/// The handlers of `run.export`, `export.save_as` and `export.stream`.
pub(crate) fn handlers() -> [Handler; 3] {
    [
        Handler::new(&RUN_EXPORT, export),
        Handler::new(&EXPORT_SAVE_AS, save_as).streaming(),
        Handler::new(&EXPORT_STREAM, stream).streaming(),
    ]
}

/// What the Records' reader hands the file's writer.
enum Part {
    /// The next bytes.
    Bytes(Vec<u8>),
    /// Every byte was handed: the file is whole.
    End,
}

/// The bytes of an Export as the file's writer reads them, from the parts
/// the Records' reader sends. A reader that stopped before its end, as
/// when the call is cancelled or the run file fails, fails the read, so
/// the engine keeps no file of a part of an Export.
struct Parts {
    receiver: mpsc::Receiver<Part>,
    current: Vec<u8>,
    read: usize,
    ended: bool,
}

impl Read for Parts {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        while self.read == self.current.len() {
            if self.ended {
                return Ok(0);
            }
            match self.receiver.blocking_recv() {
                Some(Part::Bytes(bytes)) => {
                    self.current = bytes;
                    self.read = 0;
                }
                Some(Part::End) => self.ended = true,
                None => {
                    return Err(io::Error::other(
                        "the Records' reader stopped before the Export's end",
                    ));
                }
            }
        }
        let taken = buffer.len().min(self.current.len() - self.read);
        buffer[..taken].copy_from_slice(&self.current[self.read..self.read + taken]);
        self.read += taken;
        Ok(taken)
    }
}

async fn export(params: RunExportParams, context: CallContext) -> Result<Export, CallError> {
    let options = options(&params, &context).await?;
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?;
    reader
        .read(Datasets::read)
        .await
        .map_err(failed)?
        .readable(&params.dataset)?;
    let dataset = params.dataset.clone();
    let through = reader
        .read(move |connection| records::last_number(connection, &dataset))
        .await
        .map_err(failed)?;
    let (send, receiver) = mpsc::channel(QUEUED);
    let source = Parts {
        receiver,
        current: Vec::new(),
        read: 0,
        ended: false,
    };
    let mut columns = Columns::new(options.nested(params.format));
    if params.format.is_table() {
        let mut last = 0;
        loop {
            let page = page(&mut reader, &params.dataset, last, through, params.wall)
                .await
                .map_err(failed)?;
            let full = page.len() == PAGE;
            for record in page {
                columns.see(&exported(&record).marked());
                last = record.number;
            }
            if !full {
                break;
            }
        }
    }
    let encoder = ExportEncoder::new(params.format, &options, columns);
    let compression = params.compression.unwrap_or_default();
    let reading = Reading {
        dataset: &params.dataset,
        through,
        wall: params.wall,
    };
    let (kept, written) = tokio::join!(
        context.runs().keep_export(params.run_id.clone(), source),
        encode(
            &mut reader,
            reading,
            encoder,
            Compressor::new(compression),
            send
        ),
    );
    let (records, last_number) = written.map_err(failed)?;
    let kept = match kept {
        Ok(kept) => kept,
        Err(error) => {
            let folder = context
                .runs()
                .folder(params.run_id.clone())
                .await
                .map_err(failed)?;
            return Err(keep_error(&params.run_id, &folder, error));
        }
    };
    Ok(Export {
        export_id: ExportId::of(&params.run_id, &kept.sha256),
        format: params.format,
        compression,
        records,
        last_number,
        size: kept.size,
        sha256: kept.sha256,
    })
}

/// Which Records an Export reads: its Dataset's, through a number, with
/// or without a Wall marker.
#[derive(Clone, Copy)]
struct Reading<'a> {
    dataset: &'a str,
    through: u64,
    wall: Option<bool>,
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

/// Reads the Records `reading` takes, in number order, a page at a time,
/// and sends their encoded bytes, compressed by `compressor`, to `send`,
/// then the end; answers how many it encoded and the last one's number. It
/// stops, sending no end, once the file's writer stops taking them.
async fn encode(
    reader: &mut RunReader,
    reading: Reading<'_>,
    mut encoder: ExportEncoder,
    mut compressor: Compressor,
    send: mpsc::Sender<Part>,
) -> Result<(u64, u64), RunError> {
    let mut bytes = Vec::new();
    encoder.begin(&mut bytes);
    let mut last = 0;
    loop {
        let page = page(reader, reading.dataset, last, reading.through, reading.wall).await?;
        let full = page.len() == PAGE;
        for record in &page {
            encoder.value(&exported(record), &mut bytes);
            last = record.number;
        }
        if !full {
            break;
        }
        let compressed = compressor.write(std::mem::take(&mut bytes));
        if send.send(Part::Bytes(compressed)).await.is_err() {
            return Ok((encoder.values(), last));
        }
    }
    encoder.end(&mut bytes);
    let mut compressed = compressor.write(bytes);
    compressed.append(&mut compressor.finish());
    if send.send(Part::Bytes(compressed)).await.is_ok() {
        let _ = send.send(Part::End).await;
    }
    Ok((encoder.values(), last))
}

/// The next page of `dataset`'s Records after the number `after`, through
/// the number `through`, with or without a Wall marker as `wall` says, as
/// committed.
async fn page(
    reader: &mut RunReader,
    dataset: &str,
    after: u64,
    through: u64,
    wall: Option<bool>,
) -> Result<Vec<CommittedText>, RunError> {
    let dataset = dataset.to_owned();
    reader
        .read(move |connection| {
            records::read_values(
                connection,
                Query {
                    dataset: &dataset,
                    after,
                    through: Some(through),
                    wall,
                },
                PAGE,
            )
        })
        .await
}

/// What the `export.*` settings of the call make of its format, resolved
/// in its own scope; refused before any Record is read.
async fn options(
    params: &RunExportParams,
    context: &CallContext,
) -> Result<ExportOptions, CallError> {
    let (settings, _) = context
        .runs()
        .settings()
        .open(Opening {
            run: None,
            project: params.project.clone(),
            recipe: None,
            settings: params.settings.clone(),
            origin: Some(RunOrigin::Settings),
            acting_for_model: context.surface().acts_for_model(),
        })
        .await
        .map_err(|error| settings_error(&error, context))?;
    let resolved = settings.resolved();
    let mut options = ExportOptions::of_run(params.run_id.as_str(), &params.dataset);
    match params.format {
        ExportFormat::Csv => {
            options.csv_bom = resolved.boolean(EXPORT_CSV_BOM.name).unwrap_or(false);
            if let Some(text) = resolved.value(EXPORT_CSV_DELIMITER.name).as_str() {
                options.csv_delimiter =
                    checked(resolved, &EXPORT_CSV_DELIMITER, csv_delimiter(text))?;
            }
            if resolved.value(EXPORT_CSV_NESTED.name).as_str() == Some("flatten") {
                options.csv_nested = Nested::Flatten;
            }
        }
        ExportFormat::Xlsx => {
            if let Some(text) = resolved.value(EXPORT_XLSX_SHEET.name).as_str() {
                options.xlsx_sheet = checked(resolved, &EXPORT_XLSX_SHEET, sheet_name(text))?;
            }
        }
        ExportFormat::Json => {
            if let Some(count) = resolved.value(EXPORT_JSON_INDENT.name).as_u64() {
                options.json_indent = checked(resolved, &EXPORT_JSON_INDENT, json_indent(count))?;
            }
        }
        ExportFormat::Xml => {
            if let Some(text) = resolved.value(EXPORT_XML_ROOT.name).as_str() {
                options.xml_root = checked(resolved, &EXPORT_XML_ROOT, xml_name(text))?;
            }
            if let Some(text) = resolved.value(EXPORT_XML_RECORD.name).as_str() {
                options.xml_record = checked(resolved, &EXPORT_XML_RECORD, xml_name(text))?;
            }
        }
        ExportFormat::Parquet => {
            options.parquet_codec = match resolved.value(EXPORT_PARQUET_COMPRESSION.name).as_str() {
                Some("snappy") => ParquetCodec::Snappy,
                Some("none") => ParquetCodec::None,
                _ => ParquetCodec::Zstd,
            };
        }
        ExportFormat::Rss | ExportFormat::Atom => {
            for (setting, field) in [
                (&EXPORT_FEED_TITLE, &mut options.feed.title),
                (&EXPORT_FEED_LINK, &mut options.feed.link),
                (&EXPORT_FEED_PUBLISHED, &mut options.feed.published),
                (&EXPORT_FEED_SUMMARY, &mut options.feed.summary),
            ] {
                if let Some(text) = resolved.value(setting.name).as_str() {
                    *field = checked(resolved, setting, feed_field(text))?;
                }
            }
        }
        ExportFormat::Jsonl
        | ExportFormat::HtmlTable
        | ExportFormat::Markdown
        | ExportFormat::Avro
        | ExportFormat::ArrowIpc => {}
    }
    Ok(options)
}

/// `value`, or `setting_invalid` naming `setting`, its value, the layer
/// that set it and what is wrong with it.
fn checked<T>(
    resolved: &Resolved,
    setting: &Setting,
    value: Result<T, &str>,
) -> Result<T, CallError> {
    value.map_err(|reason| {
        let given = resolved.value(setting.name);
        let layer = resolved
            .get(setting.name)
            .map_or(SettingsLayer::Run, |effective| effective.layer);
        CallError::new(
            &SETTING_INVALID,
            format!(
                "`{}` is {given} in {}, which {reason}.",
                setting.name,
                layer.as_str()
            ),
            &SettingInvalidDetails {
                key: setting.name.to_owned(),
                layer: Some(layer),
                reason: format!("{given} {reason}"),
            },
        )
    })
}

/// An Export ID no Run's folder holds.
fn no_export(export_id: &ExportId) -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        format!(
            "no Run keeps an Export `{}`: an Export ID is the one `run.export` answered",
            export_id.0
        ),
    )
}

/// The Run folder and the file of the Export `export_id`, with its size,
/// once its Run is found.
async fn kept_export(
    export_id: &ExportId,
    context: &CallContext,
) -> Result<(std::path::PathBuf, std::path::PathBuf, u64, String), CallError> {
    let Some((run_id, sha256)) = export_id.split() else {
        return Err(no_export(export_id));
    };
    let failed = |error: RunError| run_error(error, context.identity());
    let mut reader = context
        .runs()
        .reader(run_id.clone())
        .await
        .map_err(failed)?;
    reader.read(Datasets::read).await.map_err(failed)?;
    let folder = context
        .runs()
        .folder(RunId::clone(&run_id))
        .await
        .map_err(failed)?;
    let file = export_file(&folder, sha256).ok_or_else(|| no_export(export_id))?;
    let measured = file.clone();
    let size = reader
        .blocking(move || file_size(&measured))
        .await
        .map_err(|_| no_export(export_id))?;
    Ok((folder, file, size, sha256.to_owned()))
}

/// The size of the file at `path`; it blocks, so its caller runs it as the
/// engine's blocking work.
#[expect(
    clippy::disallowed_methods,
    reason = "its caller runs it as the engine's blocking work"
)]
fn file_size(path: &std::path::Path) -> std::io::Result<u64> {
    std::fs::metadata(path).map(|metadata| metadata.len())
}

async fn save_as(
    params: ExportSaveAsParams,
    context: CallContext,
) -> Result<ExportSaved, CallError> {
    let (folder, file, size, sha256) = kept_export(&params.export_id, &context).await?;
    let written = save_kept(&context, &folder, file, params.path).await?;
    Ok(ExportSaved {
        size,
        sha256,
        written,
    })
}

async fn stream(
    params: ExportStreamParams,
    context: CallContext,
) -> Result<ExportStreamed, CallError> {
    let (folder, file, size, sha256) = kept_export(&params.export_id, &context).await?;
    send_kept(&context, &folder, &file).await?;
    Ok(ExportStreamed { size, sha256 })
}
