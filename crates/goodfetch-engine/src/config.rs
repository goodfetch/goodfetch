//! The `config.*` operations every Product carries (ADRs 0028, 0031 and
//! 0083): `config.explain` answers each setting's effective value, the
//! layer that set it and the layer that locked it, from the engine's own
//! layers and the project configuration and run values a Client gives.
//! The person's entries change the machine's own layers, never project
//! configuration or a Recipe's manifest: `config.policy_read` reads the
//! Operator policy with its digest, `config.policy_write` writes it only if
//! that digest still holds, under the policy's lock file, so a hand edit or
//! another write since the read is never lost, and `config.user_write`
//! writes user-level values once the resolver accepts them. Each write is
//! an edit of the settings editor (`settings_editor`): it holds the
//! policy's lock file, replaces its file through a file renamed over it,
//! keeps every other byte, and records each key's values before and after
//! in the Audit log.

use std::io;
use std::sync::Arc;

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

use crate::audit::AuditError;
use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::run::settings_error;
use crate::settings::{Opening, Policy, SettingsError, Values, operator_policy, policy_version};
use crate::settings_editor::{self, Change, SettingsEditor};

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
    let project = params
        .project
        .map(|project| (project.path, true))
        .or(params.missing_project.map(|path| (path, false)));
    if let Some((path, found)) = project {
        files.push(goodfetch_protocol::settings::SettingsFile {
            layer: goodfetch_protocol::settings::SettingsLayer::ProjectConfiguration,
            path,
            found,
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

impl From<settings_editor::Refused> for Refused {
    fn from(refused: settings_editor::Refused) -> Self {
        match refused {
            settings_editor::Refused::Settings(error) => Self::Settings(error),
            settings_editor::Refused::File(error) => Self::File(error),
            settings_editor::Refused::Unrecorded(error) => Self::Unrecorded(error),
        }
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
    let settings = context.engine().settings();
    let path = settings.files().operator_policy().to_owned();
    let product = settings.product();
    let version = context.identity().product_version.clone();
    blocking(&context, move || {
        let text = settings_editor::read(&path)?;
        let policy = match &text {
            Some(text) => operator_policy(text, product, &version)?,
            None => Policy::default(),
        };
        Ok(OperatorPolicyRead {
            path: path.display().to_string(),
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
    let editor = SettingsEditor::of(&context);
    blocking(&context, move || {
        editor.edit(|draft| {
            let layer = SettingsLayer::OperatorPolicy;
            let current = policy_version(draft.found(layer)?.unwrap_or_default());
            let stale = current != params.digest;
            let applied = |draft: &mut settings_editor::Draft<'_>| {
                let before = draft.operator_policy()?;
                for change in &params.changes {
                    draft.set(layer, change.table.as_str(), &change.key, &change.value)?;
                }
                let after = draft.operator_policy()?;
                Ok::<_, settings_editor::Refused>((before, after))
            };
            let (before, after) = match applied(draft) {
                Ok(applied) => applied,
                Err(_) if stale => {
                    return Err(Refused::Conflict {
                        read: params.digest,
                        current,
                    });
                }
                Err(refused) => return Err(refused.into()),
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
                    return Ok(Change::Kept(PolicyWritten {
                        digest: current,
                        diff,
                    }));
                }
                return Err(Refused::Conflict {
                    read: params.digest,
                    current,
                });
            }
            let digest = policy_version(draft.text(layer)?);
            Ok(Change::Made {
                details: json!({ "digest_before": current, "digest_after": digest, "diff": diff }),
                answer: PolicyWritten { digest, diff },
                layer,
                kind: &OPERATOR_POLICY_WRITTEN,
            })
        })
    })
    .await
}

async fn user_write(
    params: UserWriteParams,
    context: CallContext,
) -> Result<UserConfigurationWritten, CallError> {
    let editor = SettingsEditor::resolving(&context).await;
    blocking(&context, move || {
        editor.edit(|draft| {
            let layer = SettingsLayer::UserConfiguration;
            draft.machine()?;
            let before = draft.user_configuration()?;
            for change in &params.changes {
                draft.set(layer, "settings", &change.key, &change.value)?;
            }
            let after = draft.user_configuration()?;
            let diff = diff(
                params.changes.iter().map(|change| (None, &change.key)),
                |_, key| value_of(&before, key),
                |_, key| value_of(&after, key),
            );
            Ok::<_, Refused>(Change::Made {
                details: json!({ "diff": diff }),
                answer: UserConfigurationWritten { diff },
                layer,
                kind: &USER_CONFIGURATION_WRITTEN,
            })
        })
    })
    .await
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
