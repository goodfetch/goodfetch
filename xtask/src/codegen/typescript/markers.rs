//! ADR 0030's run-time warning in the TypeScript layer (ADR 0099): a
//! member of a deprecated or Experimental entry opens its body with a call
//! to the hand-written `markers.ts`, which warns once per process through
//! Node's warning channel, with the text its `TSDoc` marker carries.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::super::layer::{Deprecation, Operation, Stability};

/// The hand-written module of Collie's client package whose functions give
/// ADR 0030's run-time warning of a deprecated or Experimental member.
pub(super) const MARKERS_MODULE: &str = "../markers.ts";

/// Where Foxhound's layer imports [`MARKERS_MODULE`]'s functions from.
pub(super) const CLIENT_RUNTIME: &str = "@goodfetch/collie-client/runtime";

/// The statements a member of `operation` opens its body with: ADR 0030's
/// run-time warning of each marker it carries, once per process.
pub(super) fn warnings(operation: &Operation) -> String {
    let mut text = String::new();
    if operation.stability == Stability::Experimental {
        let _ = writeln!(text, "    warnExperimental({:?});", operation.name);
    }
    if let Some(deprecated) = &operation.deprecated {
        let _ = writeln!(
            text,
            "    warnDeprecated({:?}, {:?});",
            operation.name,
            deprecation_message(deprecated)
        );
    }
    text
}

/// The import of the [`warnings`] members of `operations` call, from
/// `module`, or nothing.
pub(super) fn markers_import<'a>(
    operations: impl IntoIterator<Item = &'a Operation>,
    module: &str,
) -> String {
    let mut names = BTreeSet::new();
    for operation in operations {
        if operation.stability == Stability::Experimental {
            names.insert("warnExperimental");
        }
        if operation.deprecated.is_some() {
            names.insert("warnDeprecated");
        }
    }
    if names.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = names.into_iter().collect();
    format!("import {{ {} }} from \"{module}\";\n", names.join(", "))
}

/// A deprecation's one text, which its `@deprecated` and its run-time
/// warning share.
pub(super) fn deprecation_message(deprecated: &Deprecation) -> String {
    format!("Since {}: {}", deprecated.since, deprecated.note)
}
