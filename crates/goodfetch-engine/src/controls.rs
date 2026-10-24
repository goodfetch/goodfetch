//! Switchable controls and their reporting (ADRs 0016, 0028 and 0102): the
//! settings each composition registers as a control that costs speed, the
//! engine's own the seven `security.*` keys, and Foxhound's composition any
//! it adds. A Run that starts with one of them switched off, which only a
//! layer the Operator policy or the Recipe did not lock or cap can do, is
//! reported once: one chained `security_switch_turned_off` entry per
//! control in the Audit log, one warning through the host's logging, and
//! the control in its Run report's `weakened`, as every setting whose value
//! weakens is. With `security.audit_chain` off, the Run's other entries are
//! written unchained.

use goodfetch_protocol::audit::{Actor, SECURITY_SWITCH_TURNED_OFF};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::security::SECURITY_AUDIT_CHAIN;
use goodfetch_protocol::settings::WeakenedSetting;
use jiff::Timestamp;
use serde_json::json;

use crate::audit::{AuditError, AuditLog, Origin};
use crate::settings::Resolved;

/// The switchable controls an engine reports, as its composition registers
/// them.
#[derive(Clone, Debug, Default)]
pub struct Controls(Vec<&'static str>);

impl Controls {
    /// The same, reporting `setting` too: a key whose value, where it
    /// weakens, switches a control off.
    #[must_use]
    pub fn with(mut self, setting: &'static Setting) -> Self {
        debug_assert!(
            setting.weakens.is_some(),
            "a switchable control's setting says which values switch it off"
        );
        if !self.0.contains(&setting.name) {
            self.0.push(setting.name);
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
                .filter(|weakened| self.0.contains(&weakened.key.as_str()))
                .collect(),
        )
    }
}

/// The controls a Run's settings switch off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwitchedOff(Vec<WeakenedSetting>);

impl SwitchedOff {
    /// Whether the Audit log's hash chain is among them, so that the Run's
    /// entries are written unchained.
    pub fn unchains(&self) -> bool {
        self.0
            .iter()
            .any(|switched| switched.key == SECURITY_AUDIT_CHAIN.name)
    }

    /// Reports each one for the new Run `run_id`, once: a chained entry in
    /// `audit`, recorded at `now`, and a warning through the host's
    /// logging. Blocks on the Audit log.
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
        for switched in &self.0 {
            tracing::warn!(
                run_id = %run_id,
                setting = %switched.key,
                layer = switched.layer.as_str(),
                "the Run `{run_id}` runs with the security control `{}` switched off by its {} layer",
                switched.key,
                switched.layer.as_str(),
            );
            audit.record_chained(
                &Origin {
                    actor: Actor::Engine,
                    surface: None,
                    layer: Some(switched.layer),
                    run_id: Some(run_id.clone()),
                },
                &SECURITY_SWITCH_TURNED_OFF,
                json!({ "setting": switched.key, "value": switched.value }),
                now,
            )?;
        }
        Ok(())
    }
}
