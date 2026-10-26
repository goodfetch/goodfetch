//! A Dataset's Export (ADRs 0103, 0116, 0119 and 0122): `run.export` reads
//! the number of the Dataset's last committed Record when the call is
//! made, then streams the Records up to it, in number order, a page at a
//! time on the Run reader's read-only connection, through the encoder into
//! the engine's keeping of an Export, which writes them whole into the
//! Run's folder or not at all; so it serves a live Run and one that ended,
//! and Records committed meanwhile are not in it. `export.save_as` and
//! `export.stream` read that file back through the engine's saving and
//! sending of a kept file.

use std::io::{self, Read};

use goodfetch_engine::run::run_error;
use goodfetch_engine::run_files::export_file;
use goodfetch_engine::run_files_operations::{keep_error, save_kept, send_kept};
use goodfetch_engine::runs::{RunError, RunReader};
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_protocol::foxhound::export::{
    EXPORT_SAVE_AS, EXPORT_STREAM, Export, ExportId, ExportSaveAsParams, ExportSaved,
    ExportStreamParams, ExportStreamed, RUN_EXPORT, RunExportParams,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::RunId;
use tokio::sync::mpsc;

use crate::declaration::Datasets;
use crate::encoding::ExportEncoder;
use crate::records::{self, Query};

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
    let encoder = ExportEncoder::new(params.format);
    let (kept, written) = tokio::join!(
        context.runs().keep_export(params.run_id.clone(), source),
        encode(&mut reader, &params.dataset, through, encoder, send),
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
        compression: params.compression,
        records,
        last_number,
        size: kept.size,
        sha256: kept.sha256,
    })
}

/// Reads `dataset`'s Records through the number `through`, in number
/// order, a page at a time, and sends their encoded bytes to `send`, then
/// the end; answers how many it encoded and the last one's number. It
/// stops, sending no end, once the file's writer stops taking them.
async fn encode(
    reader: &mut RunReader,
    dataset: &str,
    through: u64,
    mut encoder: ExportEncoder,
    send: mpsc::Sender<Part>,
) -> Result<(u64, u64), RunError> {
    let mut bytes = Vec::new();
    encoder.begin(&mut bytes);
    let mut last = 0;
    loop {
        let dataset = dataset.to_owned();
        let page = reader
            .read(move |connection| {
                records::read_values(
                    connection,
                    Query {
                        dataset: &dataset,
                        after: last,
                        through: Some(through),
                    },
                    PAGE,
                )
            })
            .await?;
        let full = page.len() == PAGE;
        for (number, value) in page {
            encoder.value(&value, &mut bytes);
            last = number;
        }
        if !full {
            break;
        }
        if send
            .send(Part::Bytes(std::mem::take(&mut bytes)))
            .await
            .is_err()
        {
            return Ok((encoder.values(), last));
        }
    }
    encoder.end(&mut bytes);
    if send.send(Part::Bytes(bytes)).await.is_ok() {
        let _ = send.send(Part::End).await;
    }
    Ok((encoder.values(), last))
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
    let size = tokio::fs::metadata(&file)
        .await
        .map_err(|_| no_export(export_id))?
        .len();
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
