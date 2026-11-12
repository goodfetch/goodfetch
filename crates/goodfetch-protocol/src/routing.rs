//! The routing rule: the Product of the engine that runs each new run, from
//! who starts it, the Product its Recipe's manifest names and what its target
//! names (ADR 0096). The Stub gateway and Goodfetch Cloud's gateway both call
//! [`product_for_run`], so they cannot route differently. It reads only the
//! Product marks, through the generated table of the names only Foxhound
//! carries (ADR 0087), and does no I/O.

use crate::catalog::Product;
use crate::foxhound_names;

/// Who starts a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Starter {
    /// An SDK, the CLI, the MCP server or an Integration, whose `hello`
    /// names the Product it needs (ADR 0095).
    Client {
        /// The Product the Client's `hello` names.
        needs: Product,
    },
    /// A Schedule.
    Schedule,
    /// Goodfetch Cloud itself, such as an API call that names a target.
    GoodfetchCloud,
}

/// What a run's target names, by kind of name: what decides the Product of
/// a run a Schedule or Goodfetch Cloud starts with no Recipe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target<'a> {
    /// The operations it calls.
    pub operations: &'a [&'a str],
    /// The keys of the settings it sets.
    pub setting_keys: &'a [&'a str],
    /// The Plugin kinds it enables.
    pub plugin_kinds: &'a [&'a str],
    /// The Hook stages it enables.
    pub hook_stages: &'a [&'a str],
    /// The values it uses of a type both Products share (a Usage kind, a
    /// Limit scope, a Notification event).
    pub shared_type_values: &'a [&'a str],
}

/// A start of a run, as a gateway receives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunStart<'a> {
    /// The Product of the run the start's Run ID already names in the
    /// gateway's index, or `None` for a new Run ID.
    pub known_run: Option<Product>,
    /// Who starts it.
    pub starter: Starter,
    /// The Product its Recipe's manifest names, or `None` when it starts no
    /// Recipe.
    pub recipe: Option<Product>,
    /// What its target names.
    pub target: Target<'a>,
}

/// The Product of the engine that runs `start` (ADR 0096).
///
/// A known Run ID goes to its run, on that run's own Product's engine. A new
/// one follows the table: a Recipe that names Foxhound runs on Foxhound's
/// engine whoever starts it; a Client's run gets the Product its `hello`
/// names; a run a Schedule or Goodfetch Cloud starts gets
/// Collie's engine for a Recipe that names Collie, and with no Recipe,
/// Foxhound's engine if its target names anything only Foxhound carries,
/// Collie's otherwise.
pub fn product_for_run(start: &RunStart<'_>) -> Product {
    if let Some(product) = start.known_run {
        return product;
    }
    if start.recipe == Some(Product::Foxhound) {
        return Product::Foxhound;
    }
    match start.starter {
        Starter::Client { needs } => needs,
        Starter::Schedule | Starter::GoodfetchCloud => {
            if start.recipe.is_none() && names_something_only_foxhound_carries(&start.target) {
                Product::Foxhound
            } else {
                Product::Collie
            }
        }
    }
}

/// Whether `target` names anything the generated table lists as Foxhound's.
fn names_something_only_foxhound_carries(target: &Target<'_>) -> bool {
    let named_in =
        |names: &[&str], foxhound: &[&str]| names.iter().any(|name| foxhound.contains(name));
    named_in(target.operations, foxhound_names::OPERATIONS)
        || named_in(target.setting_keys, foxhound_names::SETTINGS)
        || named_in(target.plugin_kinds, foxhound_names::PLUGIN_KINDS)
        || named_in(target.hook_stages, foxhound_names::HOOK_STAGES)
        || named_in(
            target.shared_type_values,
            foxhound_names::SHARED_TYPE_VALUES,
        )
}

#[cfg(test)]
mod tests;
