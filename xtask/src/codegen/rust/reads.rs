//! A handle's reads in the Rust handles (issue 2798): the async handle
//! holds the answer that opened it, shared by its clones, and each read is
//! a plain method that answers a clone of that answer's field, sending
//! nothing; the blocking twin's asks its async handle.

use std::fmt::Write as _;

use anyhow::Result;

use super::index::Index;
use super::{EXPERIMENTAL, Twin, comment, deprecated_attribute, field, ident, indent, warning};
use crate::codegen::handles::Handle;
use crate::codegen::layer::{Deprecation, Stability};

/// The field a handle with reads holds the answer that opened it in.
const OPENED: &str = "opened";

/// A handle's reads, resolved against the protocol crate.
pub(super) struct ReadsModel {
    /// The path of the type of the answer that opened the handle.
    opened: String,
    members: Vec<ReadModel>,
}

struct ReadModel {
    ident: String,
    doc: String,
    warning: Option<String>,
    deprecated: Option<String>,
    /// The type it answers, the opened answer's field's.
    ty: String,
    /// The ident of that field.
    field: String,
}

impl ReadsModel {
    /// `handle`'s reads, or `None` when it has none.
    pub(super) fn new(index: &Index, handle: &Handle<'_>) -> Result<Option<Self>> {
        let Some(reads) = &handle.reads else {
            return Ok(None);
        };
        let fields = index.fields(&reads.opened)?;
        let mut members = Vec::new();
        for read in &reads.members {
            let operation = read.operation;
            let owner = format!("operation `{}`", operation.name);
            let held = field(&fields, &read.property, &owner)?;
            members.push(ReadModel {
                ident: ident(read.name, &owner)?,
                doc: doc(
                    &operation.description,
                    operation.stability,
                    operation.deprecated.as_ref(),
                ),
                warning: warning(
                    &operation.name,
                    operation.stability,
                    operation.deprecated.as_ref(),
                ),
                deprecated: operation.deprecated.as_ref().map(deprecated_attribute),
                ty: held.ty.clone(),
                field: held.ident.clone(),
            });
        }
        Ok(Some(Self {
            opened: index.type_path(&reads.opened)?,
            members,
        }))
    }

    /// The field the async handle holds the answer that opened it in.
    pub(super) fn field(&self, twin: Twin) -> Option<String> {
        match twin {
            Twin::Async => Some(format!(
                "pub(crate) {OPENED}: ::std::sync::Arc<{}>",
                self.opened
            )),
            Twin::Blocking => None,
        }
    }

    /// The reads' methods, as `twin` spells them.
    pub(super) fn members(&self, twin: Twin) -> String {
        let mut text = String::new();
        for read in &self.members {
            let _ = write!(text, "\n{}", indent(&read.doc));
            if let Some(deprecated) = &read.deprecated {
                let _ = write!(text, "    {deprecated}");
                if matches!(twin, Twin::Blocking) {
                    text.push_str(
                        "    #[expect(deprecated, reason = \"the twin of a deprecated member calls it\")]\n",
                    );
                }
            }
            let _ = writeln!(
                text,
                "    #[must_use]\n    pub fn {}(&self) -> {} {{",
                read.ident, read.ty
            );
            match twin {
                Twin::Async => {
                    if let Some(warning) = &read.warning {
                        let _ = writeln!(text, "        {warning}");
                    }
                    // `ToOwned` copies a `Copy` field and clones any
                    // other without Clippy's `clone_on_copy`, which a
                    // generator that cannot tell them apart would trip.
                    let _ = writeln!(
                        text,
                        "        ::std::borrow::ToOwned::to_owned(&self.{OPENED}.{})",
                        read.field
                    );
                }
                Twin::Blocking => {
                    let _ = writeln!(text, "        self.inner.{}()", read.ident);
                }
            }
            text.push_str("    }\n");
        }
        text
    }
}

/// The doc comment of a read: its description and a paragraph for each
/// marker it carries; it sends nothing, so it has no errors.
fn doc(description: &str, stability: Stability, deprecated: Option<&Deprecation>) -> String {
    let mut paragraphs = vec![description.trim().to_owned()];
    if stability == Stability::Experimental {
        paragraphs.push(EXPERIMENTAL.to_owned());
    }
    if let Some(deprecated) = deprecated {
        paragraphs.push(format!("Deprecated. {}", deprecated.message()));
    }
    comment(&paragraphs.join("\n\n"))
}
