//! A live Run's files (ADR 0145): one value that stores a page's body in
//! the Run's folder, through the body module, and keeps a file for the
//! program, built by the Runs' registry when the Run opens.

use std::sync::Arc;

use goodfetch_protocol::page::BodyHandle;
use goodfetch_protocol::storage::StorageLevel;
use tokio::sync::watch;

use super::{FileKind, KeepError, Kept};
use crate::body;
use crate::governor::StorageSample;
use crate::run_identity::RunIdentity;
use crate::run_state::RunState;

/// A live Run's files: its folder and its blocking work, from its
/// identity, the governor's last storage sample, and its run file's
/// writer, which records what it keeps.
#[derive(Clone)]
pub struct RunFiles {
    identity: RunIdentity,
    storage: watch::Receiver<Arc<StorageSample>>,
    state: RunState,
}

impl RunFiles {
    /// The files of the Run `identity` names, whose run file `state`
    /// writes, under the governor's storage samples `storage`.
    pub(crate) fn new(
        identity: RunIdentity,
        storage: watch::Receiver<Arc<StorageSample>>,
        state: RunState,
    ) -> Self {
        Self {
            identity,
            storage,
            state,
        }
    }

    /// The storage level of the filesystem holding the Run's folder at the
    /// governor's last storage sample, once a sample read it (ADR 0122).
    pub fn storage_level(&self) -> Option<StorageLevel> {
        self.storage
            .borrow()
            .of(self.identity.folder())
            .map(|filesystem| filesystem.level)
    }

    /// Stores `bytes` as one of the Run's page bodies, a file in its folder
    /// named by their digest, and answers its handle, which a Client on the
    /// same machine reads directly and any other through `page.body`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a temporary file beside the bodies,
    /// never a partial body.
    pub async fn store_body(&self, bytes: bytes::Bytes) -> std::io::Result<BodyHandle> {
        let folder = self.identity.folder().to_owned();
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let (body, file) = self
            .identity
            .owner()
            .blocking(move || body::store(&folder, &bytes))
            .await?;
        Ok(BodyHandle {
            run_id: self.identity.id().clone(),
            body,
            path: file.display().to_string(),
            size,
        })
    }

    /// Keeps the file `source` reads for the program, as the Run's file of
    /// `kind` named `name` (ADRs 0085 and 0116): written whole into its
    /// folder, named by its SHA-256, then recorded in its run state, so it
    /// outlives the page or Session that gave it, travels in its
    /// Checkpoints and goes only with the Run. While the Run folder's
    /// filesystem is at the `critical` storage level it fails at once,
    /// writing nothing, so the reserve stays for the Runs' stops (ADR
    /// 0122).
    ///
    /// # Errors
    ///
    /// `Storage` when the folder cannot take the file, `State` when its
    /// record cannot be committed.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a hidden temporary file beside the
    /// kept files, or a kept file no record names, never a partial one.
    pub async fn keep_file<R>(
        &self,
        kind: FileKind,
        name: String,
        source: R,
    ) -> Result<Kept, KeepError>
    where
        R: std::io::Read + Send + 'static,
    {
        if self.storage_level() == Some(StorageLevel::Critical) {
            return Err(KeepError::Storage {
                reason: "storage_full",
                os_code: None,
                message: format!(
                    "the filesystem of `{}` is at the critical storage level",
                    self.identity.folder().display()
                ),
            });
        }
        let folder = self.identity.folder().to_owned();
        let mut source = source;
        let staged = self
            .identity
            .owner()
            .blocking(move || super::stage(&folder, &mut source))
            .await
            .map_err(|error| KeepError::io(&error, "keep the file in the Run's folder"))?;
        let (sha256, size) = (staged.sha256.clone(), staged.size);
        let recorded = sha256.clone();
        // Placed in the write that records it, so a forget of the same
        // bytes ordered before it deletes no file the record names.
        let position = self
            .state
            .write(move |connection| {
                if let Err(error) = staged.place() {
                    return Ok(Err(error));
                }
                super::record(connection, kind, &name, &recorded, size).map(Ok)
            })
            .await?
            .map_err(|error| KeepError::io(&error, "keep the file in the Run's folder"))?;
        Ok(Kept {
            position,
            sha256,
            size,
        })
    }
}
