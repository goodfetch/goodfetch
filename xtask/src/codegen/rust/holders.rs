//! The handles a Rust handle holds and keeps (ADRs 0078, 0092): a value
//! every answer that opens it names another handle in holds a clone of that
//! handle, which a member named after it answers without a call, as a page
//! answers its Session; the handle of an opener that is not the held one
//! keeps each held handle its answers name, reached the first time, so
//! dropping it closes nothing, and shared after, as the Run keeps its
//! default Session. The blocking twin asks its async handle.

use anyhow::Result;

use super::index::Index;
use super::{Twin, field, ident, type_name};
use crate::codegen::handles::{Holding, Holds};

/// How an opener builds the handle the value it opens holds, in the
/// value's field `field`, such as a page's `session`.
pub(super) struct Holder {
    field: String,
    by: HeldBy,
}

/// Which handle an opener gives the value it opens to hold.
enum HeldBy {
    /// The opener's own handle, as a Session's `new_page` gives its page.
    Itself,
    /// The `handle` the opener's own keeps in its field `kept`, under the
    /// ID the result's field `carried` names, as the Run's `new_page` keeps
    /// its default Session.
    Kept {
        handle: String,
        carried: String,
        kept: String,
    },
}

impl Holder {
    /// How an opener whose result is `result` gives the value it opens
    /// what `holding` says.
    pub(super) fn of(holding: &Holding, index: &Index, result: &str, owner: &str) -> Result<Self> {
        let holds = holding.holds();
        let held = ident(holds.namespace, owner)?;
        let by = match holding {
            Holding::Builder(_) => HeldBy::Itself,
            Holding::Kept(_) => HeldBy::Kept {
                handle: type_name(holds.namespace)?,
                carried: field(&index.fields(result)?, &holds.id, owner)?
                    .ident
                    .clone(),
                kept: kept_field(&held),
            },
        };
        Ok(Self { field: held, by })
    }

    /// The async opener's body, from its call, `call`, already sent: it
    /// builds the `handle` its result's field `carried` names, holding
    /// what `holder` says.
    pub(super) fn opened(holder: Option<&Self>, call: &str, handle: &str, carried: &str) -> String {
        match holder {
            None => format!(
                "let host = self.host.clone();\n        {call}.map(move |opened| {handle} {{ host: host.child(opened.{carried}) }})"
            ),
            Some(Self {
                field,
                by: HeldBy::Itself,
            }) => format!(
                "let {field} = self.clone();\n        {call}.map(move |opened| {handle} {{ host: {field}.host.child(opened.{carried}), {field} }})"
            ),
            Some(Self {
                field,
                by:
                    HeldBy::Kept {
                        handle: held,
                        carried: named,
                        kept,
                    },
            }) => format!(
                "let host = self.host.clone();\n        let {kept} = self.{kept}.clone();\n        {call}.map(move |opened| {{\n            let {field} = {kept}.share(opened.{named}, |id| {held} {{ host: host.reached(id) }});\n            {handle} {{ host: {field}.host.child(opened.{carried}), {field} }}\n        }})"
            ),
        }
    }
}

/// The field a handle keeps the handles of the field `field` in, such as
/// the Run's `sessions`.
fn kept_field(field: &str) -> String {
    format!("{field}s")
}

/// A handle a handle holds or keeps: the ident of its field, its type and
/// its ID's type.
pub(super) struct HeldModel {
    ident: String,
    type_name: String,
    id: String,
}

impl HeldModel {
    pub(super) fn new(index: &Index, holds: &Holds, owner: &str) -> Result<Self> {
        Ok(Self {
            ident: ident(holds.namespace, owner)?,
            type_name: type_name(holds.namespace)?,
            id: index.type_path(&holds.type_name)?,
        })
    }

    /// The async handle's field that holds it.
    pub(super) fn held(&self, twin: Twin) -> Option<String> {
        match twin {
            Twin::Async => Some(format!("pub(crate) {}: {}", self.ident, self.type_name)),
            Twin::Blocking => None,
        }
    }

    /// The async handle's field that keeps each it reaches.
    pub(super) fn kept(&self, twin: Twin) -> Option<String> {
        match twin {
            Twin::Async => Some(format!(
                "pub(crate) {}: crate::host::Kept<{}, {}>",
                kept_field(&self.ident),
                self.id,
                self.type_name
            )),
            Twin::Blocking => None,
        }
    }

    /// The member that answers it, without a call, of a `namespace`.
    pub(super) fn member(&self, twin: Twin, namespace: &str) -> String {
        let Self {
            ident, type_name, ..
        } = self;
        let body = match twin {
            Twin::Async => format!("self.{ident}.clone()"),
            Twin::Blocking => format!("{type_name} {{ inner: self.inner.{ident}() }}"),
        };
        format!(
            "\n    /// Its `{ident}`, which every answer that opens a `{namespace}` names,\n    /// answered without a call: a clone of the `{ident}` that opened or told\n    /// of it, or of the one its opener keeps (ADRs 0078, 0092).\n    #[must_use]\n    pub fn {ident}(&self) -> {type_name} {{\n        {body}\n    }}\n"
        )
    }
}
