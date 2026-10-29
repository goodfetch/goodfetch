//! Switchable controls and their reporting (ADRs 0016, 0028 and 0102): the
//! settings each composition registers as a control that costs speed, the
//! engine's own the seven `security.*` keys, and Foxhound's composition any
//! it adds. A composition may register a key as a weakening of its own
//! instead, which is recorded as the Audit log kind it names, with the
//! value's built-in default: Foxhound's Politeness and Site-preferences
//! keys, as `politeness_weakened` (ADR 0086). A Run that starts with one of them switched off, which only a
//! layer the Operator policy or the Recipe did not lock or cap can do, is
//! reported once: one chained `security_switch_turned_off` entry per
//! control in the Audit log, one warning through the host's logging, and
//! the control in its Run report's `weakened`, as every setting whose value
//! weakens is. With `security.audit_chain` off, the Run's other entries are
//! written unchained. A Run whose `egress.private_ranges` lists any range
//! or host is recorded the same way, once, as `private_addresses_allowed`
//! with what it listed (ADR 0016), and its Run report's `weakened` names
//! it.

use goodfetch_protocol::audit::{Actor, PRIVATE_ADDRESSES_ALLOWED, SECURITY_SWITCH_TURNED_OFF};
use goodfetch_protocol::catalog::{Setting, SharedTypeValue};
use goodfetch_protocol::egress::EGRESS_PRIVATE_RANGES;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::security::SECURITY_AUDIT_CHAIN;
use goodfetch_protocol::settings::WeakenedSetting;
use jiff::Timestamp;
use serde_json::{Value, json};

use crate::audit::{AuditError, AuditLog, Origin};
use crate::settings::Resolved;

/// How a registered key's weakening value is recorded.
#[derive(Clone, Copy)]
enum Recorded {
    /// As a security control switched off.
    Switch,
    /// As this Audit log kind, with the value's built-in default.
    As {
        kind: &'static SharedTypeValue,
        built_in: fn() -> Value,
    },
}

/// A registered key.
#[derive(Clone, Copy)]
struct Control {
    setting: &'static Setting,
    recorded: Recorded,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let recorded = match self.recorded {
            Recorded::Switch => SECURITY_SWITCH_TURNED_OFF.name,
            Recorded::As { kind, .. } => kind.name,
        };
        formatter
            .debug_struct("Control")
            .field("setting", &self.setting.name)
            .field("recorded", &recorded)
            .finish()
    }
}

/// The switchable controls an engine reports, as its composition registers
/// them.
#[derive(Clone, Debug, Default)]
pub struct Controls(Vec<Control>);

impl Controls {
    /// The same, reporting `setting` too: a key whose value, where it
    /// weakens, switches a control off.
    #[must_use]
    pub fn with(self, setting: &'static Setting) -> Self {
        self.registering(setting, Recorded::Switch)
    }

    /// The same, recording a value of `setting` that weakens as `kind`, with
    /// the value, its built-in default `built_in` and the layer that set it.
    #[must_use]
    pub fn with_recorded(
        self,
        setting: &'static Setting,
        kind: &'static SharedTypeValue,
        built_in: fn() -> Value,
    ) -> Self {
        self.registering(setting, Recorded::As { kind, built_in })
    }

    fn registering(mut self, setting: &'static Setting, recorded: Recorded) -> Self {
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
    /// off, each with its value and the layer that set it, and its private
    /// ranges when it lists any.
    pub fn switched_off(&self, resolved: &Resolved) -> SwitchedOff {
        SwitchedOff(
            resolved
                .weakened()
                .into_iter()
                .filter_map(|weakened| {
                    if weakened.key == EGRESS_PRIVATE_RANGES.name {
                        return Some((weakened, None));
                    }
                    self.0
                        .iter()
                        .find(|control| control.setting.name == weakened.key)
                        .map(|control| (weakened, Some(*control)))
                })
                .collect(),
        )
    }
}

/// The controls a Run's settings switch off.
#[derive(Clone, Debug, Default)]
pub struct SwitchedOff(Vec<(WeakenedSetting, Option<Control>)>);

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
    /// warning through the host's logging. Blocks on the Audit log.
    ///
    /// # Errors
    ///
    /// When `audit` cannot be written; the switches recorded until then
    /// stay.
    pub fn report(
        &self,
        audit: &AuditLog,
        run_id: &RunId,
        now: Timestamp,
    ) -> Result<(), AuditError> {
        for (switched, control) in &self.0 {
            let origin = Origin {
                actor: Actor::Engine,
                surface: None,
                layer: Some(switched.layer),
                run_id: Some(run_id.clone()),
            };
            if switched.key == EGRESS_PRIVATE_RANGES.name {
                audit.record_chained(
                    &origin,
                    &PRIVATE_ADDRESSES_ALLOWED,
                    json!({ "setting": switched.key, "ranges": switched.value }),
                    now,
                )?;
                continue;
            }
            if let Some(Control {
                recorded: Recorded::As { kind, built_in },
                ..
            }) = control
            {
                let default = built_in();
                tracing::warn!(
                    run_id = %run_id,
                    setting = %switched.key,
                    layer = switched.layer.as_str(),
                    "the Run `{run_id}` presses sites harder than the built-in default: its {} layer sets `{}` to {}, the default being {default}",
                    switched.layer.as_str(),
                    switched.key,
                    switched.value,
                );
                audit.record(
                    &origin,
                    kind,
                    json!({ "setting": switched.key, "value": switched.value, "default": default }),
                    now,
                )?;
                continue;
            }
            tracing::warn!(
                run_id = %run_id,
                setting = %switched.key,
                layer = switched.layer.as_str(),
                "the Run `{run_id}` runs with the security control `{}` switched off by its {} layer",
                switched.key,
                switched.layer.as_str(),
            );
            audit.record_chained(
                &origin,
                &SECURITY_SWITCH_TURNED_OFF,
                json!({ "setting": switched.key, "value": switched.value }),
                now,
            )?;
        }
        Ok(())
    }
}
