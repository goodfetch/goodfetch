//! The runners' filters for chosen Journey variants (ADR 0063), which `just
//! journeys` hands to `just test`. Variants are chosen by Journey ID,
//! surface, milestone and area: the values of one flag add variants, and
//! different flags intersect. A retired Journey has no variant to choose.
//!
//! Each surface's runner (ADR 0063) gets one filter that matches exactly the
//! chosen variants, as its tests carry them (see `listings`):
//! - nextest (`cli`, `foxhound-cli`, `sdk-rust`, `http-listener`): a
//!   filterset on test paths, `test(/(^|::)journeys::j001::cli$/) | …`;
//! - Vitest (`sdk-typescript`): a tag filter, `(J-003 && sdk-typescript) || …`;
//! - pytest (`sdk-python`): `-m` on the `journey` marker's keywords.
//!
//! The runners of `mcp-*`, `skills`, `adapter-*` and `integration-*` are
//! their own tickets' choice, so their variants are named on stderr and left
//! out until then.

use goodfetch_protocol::journey::{JourneyId, Milestone};

use crate::runners::Runner;

use super::variants::{FOXHOUND_CLI, Variant};

/// What chooses the variants: an empty list chooses every value.
pub(super) struct Choice {
    pub(super) ids: Vec<JourneyId>,
    pub(super) surfaces: Vec<String>,
    pub(super) milestones: Vec<Milestone>,
    pub(super) areas: Vec<u8>,
}

impl Choice {
    fn chooses(&self, variant: &Variant<'_>) -> bool {
        let journey = variant.journey;
        (self.ids.is_empty() || self.ids.contains(&journey.id))
            && (self.surfaces.is_empty() || self.surfaces.contains(&variant.surface))
            && (self.milestones.is_empty() || self.milestones.contains(&variant.milestone))
            && (self.areas.is_empty() || self.areas.contains(&journey.area))
    }
}

/// The variants `choice` chooses among `variants`.
pub(super) fn chosen<'a>(variants: Vec<Variant<'a>>, choice: &Choice) -> Vec<Variant<'a>> {
    variants
        .into_iter()
        .filter(|variant| choice.chooses(variant))
        .collect()
}

/// The runner that runs a surface's variants, when one is chosen yet.
fn runner(surface: &str) -> Option<Runner> {
    match surface {
        "cli" | FOXHOUND_CLI | "sdk-rust" | "http-listener" => Some(Runner::Nextest),
        "sdk-typescript" => Some(Runner::Vitest),
        "sdk-python" => Some(Runner::Pytest),
        _ => None,
    }
}

/// Each runner's arguments, as `(runner, argument)` pairs in the runners'
/// order, and the IDs of the variants no runner runs yet.
pub(super) fn filters(variants: &[Variant<'_>]) -> (Vec<(Runner, String)>, Vec<String>) {
    let unrun = variants
        .iter()
        .filter(|variant| runner(&variant.surface).is_none())
        .map(Variant::id)
        .collect();
    let mut arguments = Vec::new();
    for runner in Runner::ALL {
        let of: Vec<&Variant<'_>> = variants
            .iter()
            .filter(|variant| self::runner(&variant.surface) == Some(runner))
            .collect();
        if of.is_empty() {
            continue;
        }
        let joined = |each: fn(&Variant<'_>) -> String, separator: &str| {
            of.iter()
                .map(|variant| each(variant))
                .collect::<Vec<_>>()
                .join(separator)
        };
        match runner {
            Runner::Nextest => {
                arguments.push((runner, format!("--filterset={}", joined(nextest, " | "))));
            }
            Runner::Vitest => {
                arguments.push((runner, format!("--tags-filter={}", joined(vitest, " || "))));
            }
            // No surface's variants run on Playwright.
            Runner::Playwright => {}
            Runner::Pytest => {
                arguments.push((runner, "-m".to_owned()));
                arguments.push((runner, joined(pytest, " or ")));
            }
        }
    }
    (arguments, unrun)
}

/// `journeys::j001::cli`, the variant's place in a nextest test's path.
fn nextest(variant: &Variant<'_>) -> String {
    format!(
        "test(/(^|::)journeys::{}::{}$/)",
        variant.journey.id.as_str().to_lowercase().replace('-', ""),
        variant.surface.replace('-', "_")
    )
}

fn vitest(variant: &Variant<'_>) -> String {
    format!("({} && {})", variant.journey.id, variant.surface)
}

fn pytest(variant: &Variant<'_>) -> String {
    format!(
        "journey(id=\"{}\", surface=\"{}\")",
        variant.journey.id, variant.surface
    )
}
