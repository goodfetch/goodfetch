//! A Dataset's Export (ADRs 0103, 0116, 0119 and 0122): `run.export` reads
//! the number of the Dataset's last committed Record when the call is
//! made, then streams the Records up to it, in number order, a page at a
//! time on the Run reader's read-only connection, through the Export's
//! pipeline into the engine's keeping of an Export, which writes them whole
//! into the Run's folder or not at all; so it serves a live Run and one that
//! ended, and Records committed meanwhile are not in it. A table format reads the
//! same Records twice: once for the columns, the fields every Record holds
//! in first-seen order, then for the rows. Its `export.*` settings resolve
//! in the call's own scope: the machine's layers, the call's project
//! configuration and its run layer. `export.save_as` and `export.stream`
//! read that file back through the engine's saving and sending of a kept
//! file.

use std::io::{self, Read};

use goodfetch_engine::run::{run_error, settings_error};
use goodfetch_engine::run_files::export_file;
use goodfetch_engine::run_files_operations::{keep_error, kept_size, save_kept, send_kept};
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

use crate::encoding::{
    ExportOptions, Nested, ParquetCodec, csv_delimiter, feed_field, json_indent, sheet_name,
    xml_name,
};
use crate::open::{open_dataset, open_run};
use crate::pipeline::{Encoding, Pipeline};
use crate::records::{self, Pages};

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
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = open_dataset(&context, params.run_id.clone(), &params.dataset).await?;
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
    let compression = params.compression.unwrap_or_default();
    let mut pipeline = Pipeline::new(params.format, options, compression);
    if pipeline.reads_columns() {
        let mut pages = Pages::new(&params.dataset, 0, Some(through), params.wall);
        loop {
            let (page, full) = pages.next_values(&mut reader).await.map_err(failed)?;
            pipeline.see(&page);
            if !full {
                break;
            }
        }
    }
    let pages = Pages::new(&params.dataset, 0, Some(through), params.wall);
    let (kept, written) = tokio::join!(
        context.runs().keep_export(params.run_id.clone(), source),
        encode(&mut reader, pages, pipeline.encoding(), send),
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

/// Reads the Records `pages` takes, a page at a time, and sends their
/// compressed bytes as `encoding` writes them to `send`, then the end;
/// answers how many it encoded and the last one's number. It stops,
/// sending no end, once the file's writer stops taking them.
async fn encode(
    reader: &mut RunReader,
    mut pages: Pages,
    mut encoding: Encoding,
    send: mpsc::Sender<Part>,
) -> Result<(u64, u64), RunError> {
    loop {
        let (page, full) = pages.next_values(reader).await?;
        encoding.write(&page);
        if !full {
            break;
        }
        if send.send(Part::Bytes(encoding.take())).await.is_err() {
            return Ok((encoding.records(), pages.after()));
        }
    }
    let encoded = (encoding.records(), pages.after());
    if send.send(Part::Bytes(encoding.finish())).await.is_ok() {
        let _ = send.send(Part::End).await;
    }
    Ok(encoded)
}

/// What the `export.*` settings of the call make of its format, resolved
/// in its own scope; refused before any Record is read.
async fn options(
    params: &RunExportParams,
    context: &CallContext,
) -> Result<ExportOptions, CallError> {
    let (settings, _) = context
        .engine()
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
            options.csv_bom = resolved.get(&EXPORT_CSV_BOM);
            let delimiter = csv_delimiter(resolved.get(&EXPORT_CSV_DELIMITER));
            options.csv_delimiter = checked(resolved, &EXPORT_CSV_DELIMITER, delimiter)?;
            options.csv_nested = Nested::of(resolved.get(&EXPORT_CSV_NESTED))
                .expect("invariant: the resolver lets only `export.csv.nested`'s choices through");
        }
        ExportFormat::Xlsx => {
            if let Some(text) = resolved.get::<Option<&str>>(&EXPORT_XLSX_SHEET) {
                options.xlsx_sheet = checked(resolved, &EXPORT_XLSX_SHEET, sheet_name(text))?;
            }
        }
        ExportFormat::Json => {
            let indent = json_indent(resolved.get(&EXPORT_JSON_INDENT));
            options.json_indent = checked(resolved, &EXPORT_JSON_INDENT, indent)?;
        }
        ExportFormat::Xml => {
            let root = xml_name(resolved.get(&EXPORT_XML_ROOT));
            options.xml_root = checked(resolved, &EXPORT_XML_ROOT, root)?;
            let record = xml_name(resolved.get(&EXPORT_XML_RECORD));
            options.xml_record = checked(resolved, &EXPORT_XML_RECORD, record)?;
        }
        ExportFormat::Parquet => {
            options.parquet_codec = ParquetCodec::of(resolved.get(&EXPORT_PARQUET_COMPRESSION))
                .expect(
                    "invariant: the resolver lets only `export.parquet.compression`'s choices through",
                );
        }
        ExportFormat::Rss | ExportFormat::Atom => {
            for (setting, field) in [
                (&EXPORT_FEED_TITLE, &mut options.feed.title),
                (&EXPORT_FEED_LINK, &mut options.feed.link),
                (&EXPORT_FEED_PUBLISHED, &mut options.feed.published),
                (&EXPORT_FEED_SUMMARY, &mut options.feed.summary),
            ] {
                *field = checked(resolved, setting, feed_field(resolved.get(setting)))?;
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
    value: Result<T, impl std::fmt::Display>,
) -> Result<T, CallError> {
    value.map_err(|reason| {
        let given = resolved.value(setting);
        let layer = resolved
            .effective(setting)
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
                limit: None,
                fix: None,
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
    let failed = |error: RunError| run_error(error, context);
    open_run(context, RunId::clone(&run_id)).await?;
    let folder = context
        .runs()
        .folder(RunId::clone(&run_id))
        .await
        .map_err(failed)?;
    let file = export_file(&folder, sha256).ok_or_else(|| no_export(export_id))?;
    let size = kept_size(context, &file)
        .await
        .map_err(|_| no_export(export_id))?;
    Ok((folder, file, size, sha256.to_owned()))
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
