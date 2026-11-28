//! A crawl's Dataset Export for a program that does not use async (ADRs 0081,
//! 0103, 0116 and 0119; issue 1488): the blocking twins of
//! [`crate::ExportDataset`] and [`crate::Export`].

use std::future::IntoFuture as _;
use std::path::Path;
use std::time::Duration;

use goodfetch_collie_client::Error;
use goodfetch_collie_client::blocking::{self, EventIter, SaveFile};
use goodfetch_protocol::foxhound::export::{
    EXPORT_SAVE_AS, ExportCompression, ExportFormat, ExportId, ExportSaveAsParams, ExportSaved,
};

use crate::settings::Settings;

/// A completed Export of one Dataset of a crawl's Run, as [`crate::Export`],
/// whose calls block the calling thread.
#[derive(Clone, Debug)]
pub struct Export {
    inner: crate::Export,
    connection: blocking::ClientConnection,
}

impl Export {
    /// Wraps an async [`crate::Export`] with its blocking connection.
    pub(super) fn new(inner: crate::Export, connection: blocking::ClientConnection) -> Self {
        Self { inner, connection }
    }

    /// Its Export ID.
    pub fn id(&self) -> &ExportId {
        self.inner.id()
    }

    /// Its format.
    pub fn format(&self) -> ExportFormat {
        self.inner.format()
    }

    /// Its compression.
    pub fn compression(&self) -> ExportCompression {
        self.inner.compression()
    }

    /// How many Records it holds.
    pub fn records(&self) -> u64 {
        self.inner.records()
    }

    /// The number of the last Record it holds, 0 when it holds none.
    pub fn last_number(&self) -> u64 {
        self.inner.last_number()
    }

    /// Its size in bytes.
    pub fn size(&self) -> u64 {
        self.inner.size()
    }

    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    pub fn sha256(&self) -> &str {
        self.inner.sha256()
    }

    /// The run epoch of the Run's state it was written from (ADR 0003).
    pub fn run_epoch(&self) -> u64 {
        self.inner.run_epoch()
    }

    /// The Dataset's fields marked with `"x-goodfetch-personal": true` in its
    /// JSON Schema, sorted (ADR 0016); empty when none are marked.
    pub fn personal_columns(&self) -> &[String] {
        self.inner.personal_columns()
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
    /// The save of this Export's file to `path`, as [`crate::Export::save_as`],
    /// which `send()` sends.
    ///
    /// # Errors
    ///
    /// Once sent, as [`crate::Export::save_as`].
    pub fn save_as(&self, path: impl AsRef<Path>) -> SaveFile<ExportSaved> {
        self.connection.save_file(
            EXPORT_SAVE_AS.name,
            &ExportSaveAsParams {
                export_id: self.inner.id().clone(),
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
    /// The Export's bytes, as [`crate::Export::stream`], as an iterator that
    /// blocks the calling thread until the next chunk.
    ///
    /// # Errors
    ///
    /// An item fails as [`crate::Export::stream`]'s does, after which the
    /// iterator ends.
    pub fn stream(&self) -> EventIter<Vec<u8>> {
        self.connection.iter(self.inner.stream())
    }
}

/// The call that writes an Export of one Dataset of a crawl's Run, as
/// [`crate::ExportDataset`], which [`ExportDataset::send`] sends.
#[derive(Debug)]
#[must_use = "an export sends nothing until `send()` is called"]
pub struct ExportDataset {
    export: crate::ExportDataset,
    connection: blocking::ClientConnection,
}

impl ExportDataset {
    /// Wraps an async [`crate::ExportDataset`] with its blocking connection.
    pub(super) fn new(
        export: crate::ExportDataset,
        connection: blocking::ClientConnection,
    ) -> Self {
        Self { export, connection }
    }

    /// Compresses the whole Export file (`none`, `gzip` or `zstd`).
    pub fn compression(mut self, compression: ExportCompression) -> Self {
        self.export = self.export.compression(compression);
        self
    }

    /// Filters by Wall Provenance (ADR 0052).
    pub fn wall(mut self, wall: bool) -> Self {
        self.export = self.export.wall(wall);
        self
    }

    /// Sets the Export's own settings (such as `export.csv.bom`).
    pub fn settings(mut self, settings: Settings) -> Self {
        self.export = self.export.settings(settings);
        self
    }

    /// Bounds the wait for `run.export` to answer.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.export = self.export.timeout(timeout);
        self
    }

    /// Sends `run.export` and blocks until the Export's file is whole.
    ///
    /// # Errors
    ///
    /// As awaiting [`crate::ExportDataset`].
    pub fn send(self) -> Result<Export, Error> {
        let connection = self.connection;
        let inner = connection.wait(self.export.into_future())?;
        Ok(Export::new(inner, connection))
    }
}
