//! A crawl's Dataset Export (ADRs 0103, 0116 and 0119; issue 1488):
//! [`CrawlRun::export`](crate::CrawlRun::export) returns an [`ExportDataset`]
//! builder that calls `run.export` and resolves to an [`Export`] only once
//! the engine's file is whole. [`Export::save_as`] writes that file to a path
//! on the program's machine whole or not at all through a hidden temporary
//! file, and [`Export::stream`] yields its bytes in order.

use std::future::IntoFuture;
use std::path::Path;
use std::time::Duration;

use goodfetch_collie_client::{ClientConnection, Error, EventStream, Reply, SaveFile};
use goodfetch_protocol::catalog::SettingScope;
use goodfetch_protocol::foxhound::export::{
    EXPORT_SAVE_AS, EXPORT_STREAM, Export as WireExport, ExportCompression, ExportFormat, ExportId,
    ExportSaveAsParams, ExportSaved, ExportStreamParams, RUN_EXPORT, RunExportParams,
};
use goodfetch_protocol::run::RunId;
use serde_json::Map;

use crate::settings::Settings;

/// A completed Export of one Dataset of a crawl's Run (ADRs 0103 and 0116):
/// its descriptor, whose `size`, `records`, `last_number` and `sha256` match
/// the whole file the engine keeps in the Run's folder, with
/// [`Export::save_as`] to place the file on the program's machine whole or
/// not at all and [`Export::stream`] to read its bytes in order.
#[derive(Clone, Debug)]
pub struct Export {
    connection: ClientConnection,
    descriptor: WireExport,
}

impl Export {
    /// Wraps a completed wire descriptor with the connection that serves its
    /// `export.save_as` and `export.stream` calls.
    pub(crate) fn new(connection: ClientConnection, descriptor: WireExport) -> Self {
        Self {
            connection,
            descriptor,
        }
    }

    /// Its Export ID.
    pub fn id(&self) -> &ExportId {
        &self.descriptor.export_id
    }

    /// Its format.
    pub fn format(&self) -> ExportFormat {
        self.descriptor.format
    }

    /// Its compression.
    pub fn compression(&self) -> ExportCompression {
        self.descriptor.compression
    }

    /// How many Records it holds.
    pub fn records(&self) -> u64 {
        self.descriptor.records
    }

    /// The number of the last Record it holds, 0 when it holds none.
    pub fn last_number(&self) -> u64 {
        self.descriptor.last_number
    }

    /// Its size in bytes.
    pub fn size(&self) -> u64 {
        self.descriptor.size
    }

    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    pub fn sha256(&self) -> &str {
        &self.descriptor.sha256
    }

    /// The run epoch of the Run's state it was written from (ADR 0003).
    pub fn run_epoch(&self) -> u64 {
        self.descriptor.run_epoch
    }

    /// Saves an Export's file, which the engine keeps in its Run's folder,
    /// whole or not at all. An Attached engine given the program's path
    /// writes it through a hidden temporary file beside it, synced and
    /// renamed over it, creating missing folders and replacing a file there;
    /// any other engine writes nothing on its own machine and sends the file
    /// in `$/chunk` notifications of at most 64 KiB, one per credit granted
    /// with `$/credit`, for the SDK to write. Answers the file's size, its
    /// SHA-256 and the run epoch the Export was written under. A path that
    /// cannot be written raises `path_not_writable`, and full or failing
    /// storage `file_write_failed`, leaving the path as it was.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// ```no_run
    /// use std::path::Path;
    /// use goodfetch_foxhound::{ConnectOptions, Crawl, ExportFormat, Foxhound, Seed};
    ///
    /// # async fn export_books() -> Result<(), goodfetch_foxhound::Error> {
    /// let foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    /// let mut crawl = Crawl::new();
    /// let page = crawl.label::<()>("page")?;
    /// let books = crawl.dataset::<serde_json::Value>("books")?;
    /// crawl.handle(&page, |_ctx| async { Ok(()) })?;
    /// let run = foxhound
    ///     .start_crawl(&crawl, [Seed::new("https://shop.test/", &page, ())])
    ///     .await?;
    /// crawl.serve(&run).await?;
    /// let export = run.export(&books, ExportFormat::Csv).await?;
    /// export.save_as(Path::new("out/books.csv")).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Once awaited: `path_not_writable` when `path` cannot hold a file,
    /// `file_write_failed` when storage fails, `closed` once the connection
    /// has ended, or the engine's error.
    pub fn save_as(&self, path: impl AsRef<Path>) -> SaveFile<ExportSaved> {
        self.connection.save_file(
            EXPORT_SAVE_AS.name,
            &ExportSaveAsParams {
                export_id: self.descriptor.export_id.clone(),
                path: None,
            },
            path.as_ref(),
            "save_as",
        )
    }

    /// Sends an Export's bytes, which the engine keeps in its Run's folder,
    /// in order, in `$/chunk` notifications of at most 64 KiB, one per credit
    /// granted with `$/credit`, and answers its size, its SHA-256 and the run
    /// epoch it was written under once every byte is sent. Nothing is written
    /// on either machine: a program that writes the bytes itself meets its
    /// own language's file errors.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    ///
    /// # Errors
    ///
    /// Each item is a decoded byte chunk; an item fails with `closed` once
    /// the connection has ended or the engine's error, after which the stream
    /// ends.
    ///
    /// # Cancel safety
    ///
    /// Dropping the stream cancels `export.stream`, leaving the Export's file
    /// in the Run's folder intact.
    pub fn stream(&self) -> EventStream<Vec<u8>> {
        self.connection.stream_chunks(
            EXPORT_STREAM.name,
            &ExportStreamParams {
                export_id: self.descriptor.export_id.clone(),
            },
        )
    }
}

/// The call that writes an Export of one Dataset of a crawl's Run, built by
/// [`CrawlRun::export`](crate::CrawlRun::export): nothing is sent until it is
/// awaited.
#[derive(Debug)]
#[must_use = "an export sends nothing until it is awaited"]
pub struct ExportDataset {
    connection: ClientConnection,
    params: RunExportParams,
    settings: Settings,
    timeout: Option<Duration>,
}

impl ExportDataset {
    /// Prepares an export of `dataset` in `run_id` in `format`.
    pub(crate) fn new(
        connection: ClientConnection,
        run_id: RunId,
        dataset: &str,
        format: ExportFormat,
    ) -> Self {
        Self {
            connection,
            params: RunExportParams {
                run_id,
                dataset: dataset.to_owned(),
                format,
                compression: None,
                wall: None,
                settings: Map::new(),
                project: None,
            },
            settings: Settings::new(),
            timeout: None,
        }
    }

    /// Compresses the whole Export file (`none`, `gzip` or `zstd`).
    pub fn compression(mut self, compression: ExportCompression) -> Self {
        self.params.compression = Some(compression);
        self
    }

    /// Filters by Wall Provenance (ADR 0052): `true` writes only Records read
    /// past a Wall, `false` only the others.
    pub fn wall(mut self, wall: bool) -> Self {
        self.params.wall = Some(wall);
        self
    }

    /// Sets the Export's own settings (such as `export.csv.bom`), over the
    /// connection's and project's settings (ADR 0083).
    pub fn settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    /// Bounds the wait for `run.export` to answer.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl IntoFuture for ExportDataset {
    type Output = Result<Export, Error>;
    type IntoFuture = Reply<Export>;

    fn into_future(self) -> Self::IntoFuture {
        let Self {
            connection,
            mut params,
            settings,
            timeout,
        } = self;
        Box::pin(async move {
            params.settings = settings.within(SettingScope::Run)?;
            params.project = connection.project().cloned();
            let mut call = connection.call::<WireExport>(RUN_EXPORT.name, &params);
            if let Some(timeout) = timeout {
                call = call.timeout(timeout);
            }
            let descriptor = call.await?;
            Ok(Export::new(connection, descriptor))
        })
    }
}
