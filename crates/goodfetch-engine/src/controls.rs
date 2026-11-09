//! Switchable controls and their reporting (ADRs 0016, 0028 and 0102): the
//! settings each composition registers as a control that costs speed, the
//! engine's own the seven `security.*` keys, and Foxhound's composition any
//! it adds. A composition may register a key as a weakening of its own
//! instead, which is recorded as the Audit log kind it names, with the
//! value's built-in default: Foxhound's Politeness and Site-preferences
//! keys, as `politeness_weakened` (ADR 0086), and `frontier.learning`, as
//! `learning_switched_off` (ADR 0138). A Run that starts with one of them switched off, which only a
//! layer the Operator policy or the Recipe did not lock or cap can do, is
//! reported once, and a resume that switches off one its report does not
//! list yet reports that one the same way: one chained `security_switch_turned_off` entry per
//! control in the Audit log, one warning through the host's logging, and
//! the control in its Run report's `weakened`, as every setting whose value
//! weakens is. With `security.audit_chain` off, the Run's other entries are
//! written unchained. A composition may also register a key whose value
//! lists what the engine refuses by default, recorded the same way, once,
//! as the kind it names with what it listed: the engine's own
//! `egress.private_ranges`, as `private_addresses_allowed` (ADR 0016),
//! which its Run report's `weakened` names too.

use goodfetch_protocol::audit::{Actor, SECURITY_SWITCH_TURNED_OFF};
use goodfetch_protocol::catalog::{Setting, SharedTypeValue};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::security::SECURITY_AUDIT_CHAIN;
use goodfetch_protocol::settings::WeakenedSetting;
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::audit::{AuditError, AuditLog, NewEntry, Origin};
use crate::settings::Resolved;

/// How a registered key's weakening value is recorded.
#[derive(Clone, Copy)]
pub(crate) enum Recorded {
    /// As a security control switched off.
    Switch,
    /// As this Audit log kind, with the value's built-in default.
    As {
        kind: &'static SharedTypeValue,
        built_in: fn() -> Value,
    },
    /// As this Audit log kind, chained whatever `security.audit_chain`
    /// says, with what the value lists under `field`: an allowance of what
    /// the engine refuses by default, such as the private ranges a Run may
    /// reach.
    Listing {
        kind: &'static SharedTypeValue,
        field: &'static str,
    },
}

impl Recorded {
    fn kind(self) -> &'static SharedTypeValue {
        match self {
            Self::Switch => &SECURITY_SWITCH_TURNED_OFF,
            Self::As { kind, .. } | Self::Listing { kind, .. } => kind,
        }
    }
}

/// A registered key.
#[derive(Clone, Copy)]
struct Control {
    setting: &'static Setting,
    recorded: Recorded,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Control")
            .field("setting", &self.setting.name)
            .field("recorded", &self.recorded.kind().name)
            .finish()
    }
}

/// The switchable controls an engine reports, as its composition registers
/// them.
#[derive(Clone, Debug, Default)]
pub struct Controls(Vec<Control>);

impl Controls {
    /// The same, reporting `setting` too: a value of it that weakens is
    /// recorded as `recorded` says, with the layer that set it. The first
    /// registration of a key holds.
    #[must_use]
    pub(crate) fn with(mut self, setting: &'static Setting, recorded: Recorded) -> Self {
        debug_assert!(
            setting.weakens.is_some(),
            "a switchable control's setting says which values switch it off"
        );
        if !self
            .0
            .iter()
            .any(|control| control.setting.name == setting.name)
        {
            self.0.push(Control { setting, recorded });
        }
        self
    }

    /// The registered controls that `resolved`, a Run's settings, switches
    /// off, each with its value and the layer that set it.
    pub fn switched_off(&self, resolved: &Resolved) -> SwitchedOff {
        SwitchedOff(
            resolved
                .weakened()
                .into_iter()
                .filter_map(|weakened| {
                    self.0
                        .iter()
                        .find(|control| control.setting.name == weakened.key)
                        .map(|control| (weakened, *control))
                })
                .collect(),
        )
    }
}

/// The controls a Run's settings switch off.
#[derive(Clone, Debug, Default)]
pub struct SwitchedOff(Vec<(WeakenedSetting, Control)>);

impl SwitchedOff {
    /// Whether the Audit log's hash chain is among them, so that the Run's
    /// entries are written unchained.
    pub fn unchains(&self) -> bool {
        self.0
            .iter()
            .any(|(switched, _)| switched.key == SECURITY_AUDIT_CHAIN.name)
    }

    /// The same, but for those `listed`, a Run report's `weakened`, already
    /// names: what a resume reports beyond its start's (ADR 0016).
    #[must_use]
    pub fn beyond(self, listed: &[WeakenedSetting]) -> Self {
        Self(
            self.0
                .into_iter()
                .filter(|(switched, _)| !listed.iter().any(|weakened| weakened.key == switched.key))
                .collect(),
        )
    }

    /// Reports each one for the new Run `run_id`, once: a chained entry in
    /// `audit`, recorded at `now`, and, for a switched-off control, a
    /// warning through the host's logging. Blocks on the Audit log, which
    /// appends the entries together, in one commit with one sync (#2110).
    ///
    /// # Errors
    ///
    /// When `audit` cannot be written; none of the switches is recorded
    /// then.
    pub fn report(
        &self,
        audit: &AuditLog,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<(), AuditError> {
        let mut entries = Vec::with_capacity(self.0.len());
        for (switched, control) in &self.0 {
            let origin = Origin {
                actor: Actor::Engine,
                surface: None,
                layer: Some(switched.layer),
                run_id: Some(run_id.clone()),
            };
            let entry = match control.recorded {
                Recorded::Listing { kind, field } => {
                    let mut details = serde_json::Map::new();
                    details.insert("setting".to_owned(), json!(switched.key));
                    details.insert(field.to_owned(), switched.value.clone());
                    NewEntry {
                        origin,
                        kind,
                        details: Value::Object(details),
                        always_chained: true,
                    }
                }
                Recorded::As { kind, built_in } => {
                    let default = built_in();
                    tracing::warn!(
                        run_id = %run_id,
                        setting = %switched.key,
                        layer = switched.layer.as_str(),
                        "the Run `{run_id}` weakens a built-in default, recorded as `{}`: its {} layer sets `{}` to {}, the default being {default}",
                        kind.name,
                        switched.layer.as_str(),
                        switched.key,
                        switched.value,
                    );
                    NewEntry {
                        origin,
                        kind,
                        details: json!({ "setting": switched.key, "value": switched.value, "default": default }),
                        always_chained: false,
                    }
                }
                Recorded::Switch => {
                    tracing::warn!(
                        run_id = %run_id,
                        setting = %switched.key,
                        layer = switched.layer.as_str(),
                        "the Run `{run_id}` runs with the security control `{}` switched off by its {} layer",
                        switched.key,
                        switched.layer.as_str(),
                    );
                    NewEntry {
                        origin,
                        kind: &SECURITY_SWITCH_TURNED_OFF,
                        details: json!({ "setting": switched.key, "value": switched.value }),
                        always_chained: true,
                    }
                }
            };
            entries.push(entry);
        }
        audit.record_together(entries, now).map(drop)
    }
}
