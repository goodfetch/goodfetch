//! A person's edit of the machine's settings files (ADRs 0028, 0031 and
//! 0098), the one way `config.*` and the "Needs you" answers change the
//! Operator policy or user configuration. An edit holds the lock file beside
//! the Operator policy from its first read to its rename, so two writes of
//! either file, from this engine or another one of the major, never
//! interleave and none loses the other's change. Under it the edit reads
//! the files as they are, sets its keys through [`set`], every other byte
//! kept, reads the changed file back and, when the editor resolves, checks
//! the machine's layers so changed under the host's defaults; it then
//! replaces the file through one renamed over it ([`durable::replace`]) and
//! records the change in the Audit log: a change it cannot record stays
//! made, and the edit answers that it is unrecorded.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Arc;

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::{ProtocolVersion, SharedTypeValue};
use goodfetch_protocol::connection::{ProductVersion, Surface};
use goodfetch_protocol::formats::{Format, OPERATOR_POLICY, USER_CONFIGURATION};
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::Value;

use crate::audit::{AuditError, AuditLog, Origin};
use crate::composition::CallContext;
use crate::durable;
use crate::settings::{
    HostDefaults, Layers, MachineLayers, Policy, Settings, SettingsError, Values, arrived_in,
    operator_policy, resolve, set, user_configuration,
};
use crate::tasks::TaskOwner;

/// The lock file beside the Operator policy, which each edit of the
/// machine's settings files holds from its first read to its rename.
pub(crate) const LOCK_FILE: &str = "operator-policy.lock";

/// Why an edit failed: each but `Unrecorded` wrote nothing.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Refused {
    /// A setting's refusal, such as an unknown key or an Operator policy's
    /// lock.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// A file could not be read or written.
    #[error("the settings file cannot be read or written: {0}")]
    File(#[from] io::Error),
    /// The change was written, but the Audit log could not record it.
    #[error("the change was written, but the Audit log cannot record it: {0}")]
    Unrecorded(#[source] AuditError),
}

/// What an edit made of the files it read.
pub(crate) enum Change<T> {
    /// Nothing: the files are left as they are and nothing is recorded.
    Kept(T),
    /// The drafted file replaces its own, and the Audit log records `kind`
    /// with `details`, from `layer`.
    Made {
        answer: T,
        layer: SettingsLayer,
        kind: &'static SharedTypeValue,
        details: Value,
    },
}

/// The change an edit made, which the Audit log records: from `layer`,
/// `kind` with `details`.
struct Made {
    layer: SettingsLayer,
    kind: &'static SharedTypeValue,
    details: Value,
}

/// One call's editor of the machine's settings files: the engine's files,
/// the Product version it writes them as, the owner whose blocking policy
/// its reads and writes run on, and the person, surface and time the Audit
/// log records.
pub(crate) struct SettingsEditor {
    owner: Arc<TaskOwner>,
    settings: Settings,
    version: ProductVersion,
    /// The host's defaults the machine's layers resolve under, when the
    /// editor resolves them before it writes.
    resolving: Option<HostDefaults>,
    audit: AuditLog,
    person: String,
    surface: Surface,
    now: jiff::Timestamp,
}

impl SettingsEditor {
    /// The editor of `context`'s call, which checks a changed file by
    /// reading it back alone: the Operator policy as `config.policy_write`
    /// writes it, which governs user configuration rather than resolving
    /// beside it.
    pub(crate) fn of(context: &CallContext) -> Self {
        let engine = context.engine();
        Self {
            owner: Arc::clone(engine.owner()),
            settings: engine.settings().clone(),
            version: context.identity().product_version.clone(),
            resolving: None,
            audit: engine.audit().clone(),
            person: context.person(),
            surface: context.surface(),
            now: engine.now(),
        }
    }

    /// The editor of `context`'s call, which also resolves the machine's
    /// layers, the changed file's included, under the host's defaults
    /// before it writes.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the search for the host's defaults to the
    /// next call.
    pub(crate) async fn resolving(context: &CallContext) -> Self {
        let host = context.engine().settings().host_defaults().await;
        Self {
            resolving: Some(host),
            ..Self::of(context)
        }
    }

    /// Edits the machine's settings files under their lock: `change` reads
    /// them and drafts its keys on the [`Draft`] it is given; when it
    /// answers [`Change::Made`], each drafted file is read back, resolved
    /// when the editor resolves and replaced; the lock released, the change
    /// is recorded, so the lock is never held while the Audit log's writer
    /// is awaited (two edits at once may then record in another order than
    /// they wrote). The reads and writes run on the engine's blocking
    /// policy.
    ///
    /// # Errors
    ///
    /// `change`'s own, and a [`Refused`] for a file that cannot be read,
    /// does not read back, does not resolve or cannot be written, nothing
    /// written then, or for a change made that the Audit log cannot record.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the change made, and recorded or not.
    pub(crate) async fn edit<T, E>(
        self,
        change: impl FnOnce(&mut Draft<'_>) -> Result<Change<T>, E> + Send + 'static,
    ) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<Refused> + Send + 'static,
    {
        let owner = Arc::clone(&self.owner);
        let (editor, written) = owner
            .blocking(move || {
                let written = self.written(change);
                (self, written)
            })
            .await;
        let (answer, made) = written?;
        if let Some(made) = made {
            editor.record(made).await?;
        }
        Ok(answer)
    }

    /// Takes the lock, drafts `change`, writes what it made and releases
    /// the lock, answering the answer and what the Audit log records.
    /// Blocking.
    fn written<T, E: From<Refused>>(
        &self,
        change: impl FnOnce(&mut Draft<'_>) -> Result<Change<T>, E>,
    ) -> Result<(T, Option<Made>), E> {
        let _lock = self.lock().map_err(Refused::File)?;
        let mut draft = Draft {
            editor: self,
            machine: None,
            operator_policy: None,
            user_configuration: None,
        };
        match change(&mut draft)? {
            Change::Kept(answer) => Ok((answer, None)),
            Change::Made {
                answer,
                layer,
                kind,
                details,
            } => {
                draft.drafted(layer)?.changed = true;
                let wrote = draft.write();
                // The engine's own write is seen at once by what reads its
                // snapshot of the machine's settings, the Resource governor
                // and the Audit log, the files it wrote before any failure
                // included (ADR 0140).
                self.settings.refresh_blocking();
                wrote?;
                Ok((
                    answer,
                    Some(Made {
                        layer,
                        kind,
                        details,
                    }),
                ))
            }
        }
    }

    /// The lock beside the Operator policy, held until it drops. Blocking.
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
    )]
    fn lock(&self) -> io::Result<File> {
        let folder = folder_of(self.settings.files().operator_policy());
        durable::create_folder(folder)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(folder.join(LOCK_FILE))?;
        file.lock()?;
        Ok(file)
    }

    /// Records the change `made`; its failure comes after the change it
    /// records, which is made.
    async fn record(&self, made: Made) -> Result<(), Refused> {
        let origin = Origin {
            actor: Actor::Person {
                name: self.person.clone(),
            },
            surface: Some(self.surface),
            layer: Some(made.layer),
            run_id: None,
        };
        self.audit
            .record(&origin, made.kind, made.details, self.now)
            .await
            .map(drop)
            .map_err(Refused::Unrecorded)
    }
}

/// The machine's settings files as one edit reads and changes them, under
/// their lock.
pub(crate) struct Draft<'a> {
    editor: &'a SettingsEditor,
    machine: Option<MachineLayers>,
    operator_policy: Option<Drafted>,
    user_configuration: Option<Drafted>,
}

/// One file's text as read, and as drafted.
struct Drafted {
    found: Option<String>,
    text: String,
    changed: bool,
}

impl Draft<'_> {
    /// The machine's layers as the files hold them before the edit, read
    /// once, as a Run reads them. Blocking.
    ///
    /// # Errors
    ///
    /// As [`Settings::machine_blocking`].
    pub(crate) fn machine(&mut self) -> Result<&MachineLayers, Refused> {
        if self.machine.is_none() {
            self.machine = Some(self.editor.settings.machine_blocking()?);
        }
        Ok(self.machine.as_ref().expect("invariant: read just above"))
    }

    /// `layer`'s file as read, or `None` when there is none. Blocking.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub(crate) fn found(&mut self, layer: SettingsLayer) -> Result<Option<&str>, Refused> {
        Ok(self.drafted(layer)?.found.as_deref())
    }

    /// `layer`'s file as drafted so far: as read, or new, before any
    /// [`Self::set`]. Blocking.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub(crate) fn text(&mut self, layer: SettingsLayer) -> Result<&str, Refused> {
        Ok(&self.drafted(layer)?.text)
    }

    /// The Operator policy as drafted so far: as read, or new, before any
    /// [`Self::set`].
    ///
    /// # Errors
    ///
    /// The refusals of [`operator_policy`].
    pub(crate) fn operator_policy(&mut self) -> Result<Policy, Refused> {
        let product = self.editor.settings.product();
        let version = self.editor.version.clone();
        let text = &self.drafted(SettingsLayer::OperatorPolicy)?.text;
        Ok(operator_policy(text, product, &version)?)
    }

    /// User configuration as drafted so far: as read, or new, before any
    /// [`Self::set`].
    ///
    /// # Errors
    ///
    /// The refusals of [`user_configuration`].
    pub(crate) fn user_configuration(&mut self) -> Result<Values, Refused> {
        let product = self.editor.settings.product();
        let version = self.editor.version.clone();
        let layer = SettingsLayer::UserConfiguration;
        let text = &self.drafted(layer)?.text;
        Ok(user_configuration(layer, text, product, &version)?)
    }

    /// Sets `key` to `value` in `table` of `layer`'s file, as [`set`] sets
    /// it: the file's lowest minor raised to the minor that added `key`, the
    /// first for a key the catalog lacks, which the file's read back refuses
    /// with `setting_unknown`.
    ///
    /// # Errors
    ///
    /// As [`set`].
    pub(crate) fn set(
        &mut self,
        layer: SettingsLayer,
        table: &str,
        key: &str,
        value: &Value,
    ) -> Result<(), Refused> {
        let version = self.editor.version.clone();
        let added_in = arrived_in(key).unwrap_or(ProtocolVersion { major: 0, minor: 1 });
        let drafted = self.drafted(layer)?;
        drafted.text = set(
            &drafted.text,
            table,
            key,
            value,
            format_of(layer),
            &version,
            added_in,
        )?;
        drafted.changed = true;
        Ok(())
    }

    /// `layer`'s file, read on first use.
    fn drafted(&mut self, layer: SettingsLayer) -> Result<&mut Drafted, Refused> {
        let files = self.editor.settings.files();
        let (slot, path) = match layer {
            SettingsLayer::OperatorPolicy => (&mut self.operator_policy, files.operator_policy()),
            _ => (&mut self.user_configuration, files.user_configuration()),
        };
        if slot.is_none() {
            let found = read(path)?;
            let text = found.clone().unwrap_or_else(|| new_file(format_of(layer)));
            *slot = Some(Drafted {
                found,
                text,
                changed: false,
            });
        }
        Ok(slot.as_mut().expect("invariant: read just above"))
    }

    /// Reads each changed file back, resolves the machine's layers so
    /// changed when the editor resolves, then replaces each changed file.
    /// Blocking.
    fn write(&mut self) -> Result<(), Refused> {
        let changed =
            |drafted: &Option<Drafted>| drafted.as_ref().is_some_and(|drafted| drafted.changed);
        let policy = changed(&self.operator_policy)
            .then(|| self.operator_policy())
            .transpose()?;
        let user = changed(&self.user_configuration)
            .then(|| self.user_configuration())
            .transpose()?;
        if let Some(host) = self.editor.resolving.clone() {
            let product = self.editor.settings.product();
            let machine = self.machine()?;
            let layers = Layers {
                host,
                operator_policy: policy.unwrap_or_else(|| machine.operator_policy.clone()),
                user_configuration: user.unwrap_or_else(|| machine.user_configuration.clone()),
                ..Layers::default()
            };
            resolve(&layers, product, product)?;
        }
        let files = self.editor.settings.files();
        for (drafted, path) in [
            (&self.operator_policy, files.operator_policy()),
            (&self.user_configuration, files.user_configuration()),
        ] {
            if let Some(drafted) = drafted.as_ref().filter(|drafted| drafted.changed) {
                durable::create_folder(folder_of(path))?;
                durable::replace(path, drafted.text.as_bytes())?;
            }
        }
        Ok(())
    }
}

/// The format of `layer`'s file.
fn format_of(layer: SettingsLayer) -> &'static Format {
    match layer {
        SettingsLayer::OperatorPolicy => &OPERATOR_POLICY,
        _ => &USER_CONFIGURATION,
    }
}

/// The folder of the file at `path`.
fn folder_of(path: &Path) -> &Path {
    path.parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// A new settings file of `format`: its version alone.
fn new_file(format: &Format) -> String {
    format!("version = \"{}\"\n", format.version)
}

/// The settings file at `path`'s text, or `None` when there is none.
/// Blocking.
///
/// # Errors
///
/// When the file exists but cannot be read.
pub(crate) fn read(path: &Path) -> io::Result<Option<String>> {
    #[expect(
        clippy::disallowed_methods,
        reason = "an edit's file work runs under the engine's blocking policy"
    )]
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
