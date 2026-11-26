//! The containment check (ADR 0094): every generated contract of Collie's is
//! in Foxhound's, unchanged. It reads the two Products' projections that
//! `xtask codegen` commits, section by section, so a contract that joins the
//! catalog (setting keys, CLI commands with their flags, MCP tools) joins the
//! check, and compares each of Collie's entries, and each type they reach,
//! with Foxhound's of the same name, and of the same type for a shared
//! type's value.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{Map, Value};

use crate::check::Finding;

const COLLIE: &str = "crates/goodfetch-protocol/generated/collie.json";
const FOXHOUND: &str = "crates/goodfetch-protocol/generated/foxhound.json";

/// The section of a projection that holds its types.
const DEFINITIONS: &str = "$defs";

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let collie = read(&root.join(COLLIE))?;
    let foxhound = read(&root.join(FOXHOUND))?;
    let mut findings = Vec::new();
    for (section, collie_value) in &collie {
        let foxhound_value = foxhound.get(section);
        if section == DEFINITIONS {
            compare_definitions(collie_value, foxhound_value, &mut findings);
        } else if let Value::Array(entries) = collie_value {
            compare_entries(section, entries, foxhound_value, &mut findings);
        }
    }
    Ok(findings)
}

fn read(path: &Path) -> Result<Map<String, Value>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
}

fn compare_entries(
    section: &str,
    entries: &[Value],
    foxhound: Option<&Value>,
    findings: &mut Vec<Finding>,
) {
    let foxhound_entries = foxhound.and_then(Value::as_array);
    for entry in entries {
        let key = key(entry);
        let subject = match key.kind {
            Some(kind) => format!("{section}/{kind}.{}", key.name),
            None => format!("{section}/{}", key.name),
        };
        let counterpart = foxhound_entries.and_then(|candidates| {
            candidates
                .iter()
                .find(|candidate| self::key(candidate) == key)
        });
        compare(&subject, entry, counterpart, findings);
    }
}

/// What names an entry in its section: its `name`, within its `type` when
/// it has one, since two shared types may each hold a value of one name.
#[derive(PartialEq, Eq)]
struct Key<'a> {
    kind: Option<&'a str>,
    name: &'a str,
}

fn key(entry: &Value) -> Key<'_> {
    Key {
        kind: entry.get("type").and_then(Value::as_str),
        name: entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    }
}

fn compare_definitions(collie: &Value, foxhound: Option<&Value>, findings: &mut Vec<Finding>) {
    let Some(definitions) = collie.as_object() else {
        return;
    };
    for (name, schema) in definitions {
        let counterpart = foxhound.and_then(|definitions| definitions.get(name));
        compare(
            &format!("{DEFINITIONS}/{name}"),
            schema,
            counterpart,
            findings,
        );
    }
}

fn compare(subject: &str, collie: &Value, foxhound: Option<&Value>, findings: &mut Vec<Finding>) {
    match foxhound {
        None => findings.push(Finding::new(
            "missing-from-foxhound",
            subject,
            "Collie carries it and Foxhound's generated contracts do not (ADR 0094)",
        )),
        Some(foxhound) if foxhound != collie => findings.push(Finding::new(
            "changed-in-foxhound",
            subject,
            "Foxhound's generated contract differs from Collie's (ADR 0094)",
        )),
        Some(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::compare_entries;

    fn subjects(collie: &Value, foxhound: &Value) -> Vec<String> {
        let mut findings = Vec::new();
        compare_entries(
            "shared_type_values",
            collie.as_array().expect("an array"),
            Some(foxhound),
            &mut findings,
        );
        findings
            .into_iter()
            .map(|finding| format!("{} {}", finding.kind, finding.subject))
            .collect()
    }

    #[test]
    fn a_value_name_two_types_share_matches_within_its_type() {
        let audit =
            json!({ "name": "effect_reconciled", "type": "audit_kind", "description": "audit" });
        let event = json!({ "name": "effect_reconciled", "type": "notification_event", "description": "event" });
        let collie = json!([audit, event]);
        let foxhound = json!([event, audit]);

        assert_eq!(subjects(&collie, &foxhound), Vec::<String>::new());
    }

    #[test]
    fn a_changed_value_names_its_type() {
        let collie =
            json!([{ "name": "effect_reconciled", "type": "audit_kind", "description": "audit" }]);
        let foxhound = json!([{ "name": "effect_reconciled", "type": "audit_kind", "description": "changed" }]);

        assert_eq!(
            subjects(&collie, &foxhound),
            vec!["changed-in-foxhound shared_type_values/audit_kind.effect_reconciled"]
        );
    }
}
