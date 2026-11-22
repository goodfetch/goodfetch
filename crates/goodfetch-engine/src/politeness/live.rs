//! The Politeness a live Run shares (ADRs 0053, 0138 and 0145): the
//! per-user Politeness file the engine's Runs share, or, under
//! `politeness.sharing = run`, one of the Run's own in memory, opened on
//! the Run's first ask, with the holder its Permits name. The Runs'
//! registry builds it when the Run opens, with what watches the file's
//! storage failures, which stays the registry's: it sets no stop signal
//! itself (ADR 0145's note of 2026-11-10).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::politeness::{POLITENESS_DELEGATION, POLITENESS_SHARING};
use tokio::sync::{OnceCell, watch};

use super::{
    Holder, HolderGone, OwnPoliteness, Politeness, PolitenessError, PolitenessOptions,
    PolitenessTables,
};
use crate::settings::Resolved;
use crate::tasks::TaskOwner;

/// What the Runs' registry does with the storage failures of the
/// Politeness file a Run opened, naming the file's folder: it watches
/// them, and stops the Run at its next safe point at the first (ADR 0122).
pub type OnStorageFailures =
    Box<dyn Fn(watch::Receiver<Option<String>>, Option<&Path>) + Send + Sync>;

/// The per-user Politeness file the engine's Runs share, opened the first
/// time one shares it (ADR 0053), as one engine of the user.
pub struct SharedPoliteness {
    file: OnceCell<Politeness>,
    path: PathBuf,
    options: PolitenessOptions,
    tables: Vec<PolitenessTables>,
    holder_gone: HolderGone,
    engine: String,
    /// The engine's Politeness store, whose tasks serve each Politeness in
    /// memory: not a Run's, since a Run keeps its Politeness while it ends,
    /// and its owner waits for its tasks, a file's actor among them.
    owner: TaskOwner,
}

/// What the file the engine's Runs share opens from.
pub struct SharedPolitenessParts {
    /// The file.
    pub path: PathBuf,
    /// Its options, which a Politeness of a Run's own takes too.
    pub options: PolitenessOptions,
    /// The tables a Product's composition adds to it.
    pub tables: Vec<PolitenessTables>,
    /// Whether the Run that took Permits is gone.
    pub holder_gone: HolderGone,
    /// The engine's name in it.
    pub engine: String,
    /// The engine's Politeness store.
    pub owner: TaskOwner,
}

impl SharedPoliteness {
    /// The file `parts` name, opened on the first Run's ask.
    pub fn new(parts: SharedPolitenessParts) -> Self {
        Self {
            file: OnceCell::new(),
            path: parts.path,
            options: parts.options,
            tables: parts.tables,
            holder_gone: parts.holder_gone,
            engine: parts.engine,
            owner: parts.owner,
        }
    }

    /// The file, opened now if it is not yet.
    async fn file(&self) -> Result<Politeness, PolitenessError> {
        self.file
            .get_or_try_init(|| {
                Politeness::open(
                    self.path.clone(),
                    &self.options,
                    self.tables.clone(),
                    self.holder_gone,
                    self.engine.clone(),
                    &self.owner,
                )
            })
            .await
            .cloned()
    }
}

/// A Run's Politeness of its own (`politeness.sharing = run`), as its
/// admission comes to hold it (#2171).
enum OwnHolding {
    /// Its admission has not opened: one opened now waits for it.
    Before,
    /// Opened, waiting for its admission to hold it.
    Waiting(Box<OwnPoliteness>),
    /// Its admission opened, or its driver opens none: one opened now is
    /// served by an actor.
    After,
}

/// The Politeness a live Run shares, once it asked for it, and the holder
/// its Permits name.
pub struct RunPoliteness {
    shared: Arc<SharedPoliteness>,
    opened: OnceCell<Politeness>,
    /// The Politeness of its own that its admission comes to hold.
    own: tokio::sync::Mutex<OwnHolding>,
    run_id: String,
    epoch: u64,
    folder: PathBuf,
    on_storage_failures: OnStorageFailures,
}

/// What a live Run's Politeness is built from.
pub struct RunPolitenessParts {
    /// The file the engine's Runs share.
    pub shared: Arc<SharedPoliteness>,
    /// The Run's ID, which its Permits name.
    pub run_id: String,
    /// Its run epoch, which its Permits name.
    pub epoch: u64,
    /// Its folder, which its Permits name.
    pub folder: PathBuf,
    /// Whether its driver opens an admission, which comes to hold a
    /// Politeness of its own.
    pub admits: bool,
    /// What watches the storage failures of the file once it opens.
    pub on_storage_failures: OnStorageFailures,
}

impl RunPoliteness {
    /// A Run's, which has asked for none yet.
    pub fn new(parts: RunPolitenessParts) -> Self {
        Self {
            shared: parts.shared,
            opened: OnceCell::new(),
            own: tokio::sync::Mutex::new(if parts.admits {
                OwnHolding::Before
            } else {
                OwnHolding::After
            }),
            run_id: parts.run_id,
            epoch: parts.epoch,
            folder: parts.folder,
            on_storage_failures: parts.on_storage_failures,
        }
    }

    /// The Politeness the Run's own code shares for keys outside
    /// admission, such as a proxy's (ADRs 0053 and 0088), and the holder
    /// its Permits name: the per-user Politeness file, or, when `settings`
    /// set `politeness.sharing` to `run`, one of its own in memory. Each
    /// ask answers the one its first opened.
    ///
    /// # Errors
    ///
    /// When the Politeness file cannot be opened.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub async fn open(&self, settings: &Resolved) -> Result<(Politeness, Holder), PolitenessError> {
        let own = settings.get::<&str>(&POLITENESS_SHARING) == "run";
        let politeness = self
            .opened
            .get_or_try_init(|| async {
                if own {
                    let mut holding = self.own.lock().await;
                    if let OwnHolding::Before = *holding {
                        // Its admission holds it once it opens, which its
                        // driver does before any of its tasks waits on it.
                        let (politeness, own) = Politeness::own(&self.shared.options);
                        *holding = OwnHolding::Waiting(Box::new(own));
                        return Ok(politeness);
                    }
                    return Politeness::in_memory(&self.shared.options, &self.shared.owner);
                }
                let file = self.shared.file().await?;
                (self.on_storage_failures)(file.storage_failures(), self.shared.path.parent());
                Ok(file)
            })
            .await?
            .clone();
        let holder = Holder {
            run_id: self.run_id.clone(),
            epoch: self.epoch,
            run_folder: self.folder.clone(),
            delegates: settings.get::<bool>(&POLITENESS_DELEGATION),
        };
        Ok((politeness, holder))
    }

    /// The Politeness of its own its admission comes to hold, as it opens,
    /// if one was opened before: from then on one opened is served by an
    /// actor.
    pub(crate) async fn held_by_admission(&self) -> Option<OwnPoliteness> {
        match std::mem::replace(&mut *self.own.lock().await, OwnHolding::After) {
            OwnHolding::Waiting(own) => Some(*own),
            OwnHolding::Before | OwnHolding::After => None,
        }
    }

    /// Serves `own`, a Politeness of its own its admission does not hold,
    /// by an actor of the engine's Politeness store.
    ///
    /// # Errors
    ///
    /// When the actor cannot start.
    pub(crate) fn serve(&self, own: OwnPoliteness) -> Result<(), PolitenessError> {
        own.serve_on(&self.shared.owner)
    }
}
