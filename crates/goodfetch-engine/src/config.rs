//! The `config.*` operations every Product carries (ADRs 0028, 0031 and
//! 0083): `config.explain` answers each setting's effective value, the
//! layer that set it and the layer that locked it, from the engine's own
//! layers and the project configuration and run values a Client gives.
//! The person's entries change the machine's own layers, never project
//! configuration or a Recipe's manifest: `config.policy_read` reads the
//! Operator policy with its digest, `config.policy_write` writes it only if
//! that digest still holds, under the policy's lock file, so a hand edit or
//! another write since the read is never lost, and `config.user_write`
//! writes user-level values once the resolver accepts them. Each write
//! replaces its file through a file renamed over it, keeps every other byte,
//! and records each key's values before and after in the Audit log.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::{Format, OPERATOR_POLICY, USER_CONFIGURATION};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::edit::{
    CONFIG_POLICY_READ, CONFIG_POLICY_WRITE, CONFIG_USER_WRITE, OPERATOR_POLICY_WRITTEN,
    OperatorPolicyRead, POLICY_CONFLICT, PolicyConflictDetails, PolicyEntry, PolicyReadParams,
    PolicyTable, PolicyWriteParams, PolicyWritten, SettingDiff, USER_CONFIGURATION_WRITTEN,
    UserConfigurationWritten, UserWriteParams,
};
use goodfetch_protocol::settings::{
    CONFIG_EXPLAIN, ConfigExplainParams, RunOrigin, SettingsExplanation, SettingsLayer,
};
use serde_json::{Value, json};

use crate::audit::{AuditError, AuditLog, Origin};
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::durable;
use crate::run::settings_error;
use crate::settings::{
    HostDefaults, Layers, Opening, Policy, SettingsError, Values, operator_policy, policy_version,
    resolve, set, user_configuration,
};

/// The lock file beside the Operator policy, which each write holds from
/// its digest's check to its rename.
const POLICY_LOCK_FILE: &str = "operator-policy.lock";

/// The `config.*` handlers.
pub(crate) fn handlers() -> [Handler; 4] {
    [
        Handler::new(&CONFIG_EXPLAIN, explain),
        Handler::new(&CONFIG_POLICY_READ, policy_read),
        Handler::new(&CONFIG_POLICY_WRITE, policy_write),
        Handler::new(&CONFIG_USER_WRITE, user_write),
    ]
}

async fn explain(
    params: ConfigExplainParams,
    context: CallContext,
) -> Result<SettingsExplanation, CallError> {
    let settings = context.engine().settings();
    let (run, machine) = settings
        .open(Opening {
            run: None,
            project: params.project.clone(),
            recipe: None,
            settings: params.settings,
            origin: Some(params.origin.unwrap_or(RunOrigin::Settings)),
            acting_for_model: context.surface().acts_for_model(),
        })
        .await
        .map_err(|error| settings_error(&error, &context))?;
    let mut files = machine.files;
    if let Some(project) = params.project {
        files.push(goodfetch_protocol::settings::SettingsFile {
            layer: goodfetch_protocol::settings::SettingsLayer::ProjectConfiguration,
            path: project.path,
            found: true,
        });
    }
    Ok(run.resolved().explain(&params.keys, files))
}

/// Why a person's entry failed: each but `Unrecorded` wrote nothing.
enum Refused {
    /// A setting's refusal, such as an unknown key or an Operator policy's
    /// lock.
    Settings(SettingsError),
    /// The Operator policy's digest is no longer the one read: it is this.
    Conflict { read: String, current: String },
    /// A file could not be read or written.
    File(io::Error),
    /// The entry was written, but the Audit log could not record it.
    Unrecorded(AuditError),
}

impl From<SettingsError> for Refused {
    fn from(error: SettingsError) -> Self {
        Self::Settings(error)
    }
}

impl From<io::Error> for Refused {
    fn from(error: io::Error) -> Self {
        Self::File(error)
    }
}

/// What a person's entry works with, in its blocking part.
struct Edit {
    path: PathBuf,
    product: Product,
    version: ProductVersion,
    audit: AuditLog,
    origin: Origin,
    now: jiff::Timestamp,
}

impl Edit {
    /// What an entry of `context` writing `layer` works with.
    fn of(context: &CallContext, layer: SettingsLayer) -> Self {
        let engine = context.engine();
        let settings = engine.settings();
        let files = settings.files();
        Self {
            path: match layer {
                SettingsLayer::OperatorPolicy => files.operator_policy().to_owned(),
                _ => files.user_configuration().to_owned(),
            },
            product: settings.product(),
            version: context.identity().product_version.clone(),
            audit: engine.audit().clone(),
            origin: Origin {
                actor: Actor::Person {
                    name: context.person(),
                },
                surface: Some(context.surface()),
                layer: Some(layer),
                run_id: None,
            },
            now: engine.now(),
        }
    }

    /// Records `details` under `kind` in the Audit log; its failure comes
    /// after the write it records, which is made.
    fn record(
        &self,
        kind: &goodfetch_protocol::catalog::SharedTypeValue,
        details: Value,
    ) -> Result<(), Refused> {
        self.audit
            .record(&self.origin, kind, details, self.now)
            .map(drop)
            .map_err(Refused::Unrecorded)
    }
}

/// Runs `work` on the engine's blocking policy, its refusal as the call's
/// error.
async fn blocking<T: Send + 'static>(
    context: &CallContext,
    work: impl FnOnce() -> Result<T, Refused> + Send + 'static,
) -> Result<T, CallError> {
    let engine = context.engine();
    let owner = Arc::clone(engine.owner());
    owner.blocking(work).await.map_err(|refused| match refused {
        Refused::Settings(error) => {
            settings_error(&engine.settings().files().named_in(error), context)
        }
        Refused::Conflict { read, current } => CallError::new(
            &POLICY_CONFLICT,
            format!(
                "the Operator policy changed since it was read: its digest is `{current}`, not `{read}`, so nothing was written"
            ),
            &PolicyConflictDetails {
                digest: read,
                current,
                fix: "read the Operator policy again with `config.policy_read`, then make the changes against what it holds now".to_owned(),
            },
        ),
        Refused::File(error) => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the settings file cannot be read or written: {error}"),
        ),
        Refused::Unrecorded(error) => CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the settings file was written, but the Audit log cannot record it: {error}"),
        ),
    })
}

async fn policy_read(
    _: PolicyReadParams,
    context: CallContext,
) -> Result<OperatorPolicyRead, CallError> {
    let edit = Edit::of(&context, SettingsLayer::OperatorPolicy);
    blocking(&context, move || {
        let text = read(&edit.path)?;
        let policy = match &text {
            Some(text) => operator_policy(text, edit.product, &edit.version)?,
            None => Policy::default(),
        };
        Ok(OperatorPolicyRead {
            path: edit.path.display().to_string(),
            found: text.is_some(),
            digest: policy_version(text.as_deref().unwrap_or_default()),
            entries: entries(&policy),
        })
    })
    .await
}

async fn policy_write(
    params: PolicyWriteParams,
    context: CallContext,
) -> Result<PolicyWritten, CallError> {
    let edit = Edit::of(&context, SettingsLayer::OperatorPolicy);
    blocking(&context, move || {
        let _lock = lock(&edit.path)?;
        let text = read(&edit.path)?;
        let current = policy_version(text.as_deref().unwrap_or_default());
        let text = text.unwrap_or_else(|| new_file(&OPERATOR_POLICY));
        let applied = |text: String| -> Result<_, Refused> {
            let before = operator_policy(&text, edit.product, &edit.version)?;
            let mut written = text;
            for change in &params.changes {
                written = set(
                    &written,
                    change.table.as_str(),
                    &change.key,
                    &change.value,
                    &OPERATOR_POLICY,
                    &edit.version,
                    added_in(&change.key),
                )?;
            }
            let after = operator_policy(&written, edit.product, &edit.version)?;
            Ok((before, written, after))
        };
        let stale = current != params.digest;
        let (before, written, after) = match applied(text) {
            Ok(applied) => applied,
            Err(_) if stale => {
                return Err(Refused::Conflict {
                    read: params.digest,
                    current,
                });
            }
            Err(refused) => return Err(refused),
        };
        let diff = diff(
            params
                .changes
                .iter()
                .map(|change| (Some(change.table), &change.key)),
            |table, key| table.and_then(|table| entry(&before, table, key)),
            |table, key| table.and_then(|table| entry(&after, table, key)),
        );
        if stale {
            // A write the file already holds, as when a Client resent it
            // after its Connection dropped before the reply, was made:
            // answered as written, the file left as it is (#874).
            if entries(&after) == entries(&before) {
                return Ok(PolicyWritten {
                    digest: current,
                    diff,
                });
            }
            return Err(Refused::Conflict {
                read: params.digest,
                current,
            });
        }
        replace(&edit.path, &written)?;
        let digest = policy_version(&written);
        edit.record(
            &OPERATOR_POLICY_WRITTEN,
            json!({ "digest_before": current, "digest_after": digest, "diff": diff }),
        )?;
        Ok(PolicyWritten { digest, diff })
    })
    .await
}

async fn user_write(
    params: UserWriteParams,
    context: CallContext,
) -> Result<UserConfigurationWritten, CallError> {
    let settings = context.engine().settings().clone();
    let machine = settings
        .machine()
        .await
        .map_err(|error| settings_error(&error, &context))?;
    let host: HostDefaults = settings.host_defaults().await;
    let edit = Edit::of(&context, SettingsLayer::UserConfiguration);
    blocking(&context, move || {
        let layer = SettingsLayer::UserConfiguration;
        let text = read(&edit.path)?.unwrap_or_else(|| new_file(&USER_CONFIGURATION));
        let before = user_configuration(layer, &text, edit.product, &edit.version)?;
        let mut written = text;
        for change in &params.changes {
            written = set(
                &written,
                "settings",
                &change.key,
                &change.value,
                &USER_CONFIGURATION,
                &edit.version,
                added_in(&change.key),
            )?;
        }
        let after = user_configuration(layer, &written, edit.product, &edit.version)?;
        resolve(
            &Layers {
                host,
                operator_policy: machine.operator_policy,
                user_configuration: after.clone(),
                ..Layers::default()
            },
            edit.product,
            edit.product,
        )?;
        let diff = diff(
            params.changes.iter().map(|change| (None, &change.key)),
            |_, key| value_of(&before, key),
            |_, key| value_of(&after, key),
        );
        replace(&edit.path, &written)?;
        edit.record(&USER_CONFIGURATION_WRITTEN, json!({ "diff": diff }))?;
        Ok(UserConfigurationWritten { diff })
    })
    .await
}

/// The protocol minor that added `key`, which a write raises its file's
/// lowest minor to; an unknown key takes the first, and the file's read
/// back refuses it with `setting_unknown`.
fn added_in(key: &str) -> ProtocolVersion {
    crate::settings::arrived_in(key).unwrap_or(ProtocolVersion { major: 0, minor: 1 })
}

/// Each changed key once, in the order first changed, with its values
/// before and after, those that end as they began left out.
fn diff<'a>(
    changed: impl Iterator<Item = (Option<PolicyTable>, &'a String)>,
    before: impl Fn(Option<PolicyTable>, &str) -> Option<Value>,
    after: impl Fn(Option<PolicyTable>, &str) -> Option<Value>,
) -> Vec<SettingDiff> {
    let mut diff: Vec<SettingDiff> = Vec::new();
    for (table, key) in changed {
        if diff
            .iter()
            .any(|seen| seen.table == table && &seen.key == key)
        {
            continue;
        }
        let (before, after) = (before(table, key), after(table, key));
        if before != after {
            diff.push(SettingDiff {
                table,
                key: key.clone(),
                before: before.unwrap_or(Value::Null),
                after: after.unwrap_or(Value::Null),
            });
        }
    }
    diff
}

/// The value `values` gives `key`, applied or the other Product's.
fn value_of(values: &Values, key: &str) -> Option<Value> {
    values
        .applied
        .get(key)
        .or_else(|| values.others.get(key))
        .cloned()
}

/// The value `policy`'s `table` gives `key`.
fn entry(policy: &Policy, table: PolicyTable, key: &str) -> Option<Value> {
    value_of(table_of(policy, table), key)
}

fn table_of(policy: &Policy, table: PolicyTable) -> &Values {
    match table {
        PolicyTable::Settings => &policy.settings,
        PolicyTable::Lock => &policy.lock,
        PolicyTable::Cap => &policy.cap,
    }
}

/// Every entry of `policy`'s tables, by table, then key.
fn entries(policy: &Policy) -> Vec<PolicyEntry> {
    [PolicyTable::Settings, PolicyTable::Lock, PolicyTable::Cap]
        .into_iter()
        .flat_map(|table| {
            let values = table_of(policy, table);
            let mut keys: Vec<&'static str> = values
                .applied
                .keys()
                .chain(values.others.keys())
                .copied()
                .collect();
            keys.sort_unstable();
            keys.into_iter().filter_map(move |key| {
                value_of(values, key).map(|value| PolicyEntry {
                    table,
                    key: key.to_owned(),
                    value,
                })
            })
        })
        .collect()
}

/// A new settings file of `format`: its version alone.
fn new_file(format: &Format) -> String {
    format!("version = \"{}\"\n", format.version)
}

/// The file at `path`'s text, or `None` when there is none. Blocking.
fn read(path: &Path) -> io::Result<Option<String>> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the entries' file work runs under the engine's blocking policy"
    )]
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The Operator policy's lock, beside the policy at `path`, held until it
/// drops, so two writes never interleave their check and rename. Blocking.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn lock(path: &Path) -> io::Result<File> {
    let folder = path.parent().unwrap_or(Path::new("."));
    durable::create_folder(folder)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(POLICY_LOCK_FILE))?;
    file.lock()?;
    Ok(file)
}

/// Replaces the file at `path` with `text`, through a file renamed over
/// it, so no reader sees half of one (ADR 0002). Blocking.
fn replace(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        durable::create_folder(folder)?;
    }
    durable::replace(path, text.as_bytes())
}
