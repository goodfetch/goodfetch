//! A Journey's variants, one per surface it lists, and the rule on the tests
//! that run them (ADR 0063, rule 4). A Collie Journey that lists `cli` also
//! has `foxhound-cli`, its `cli` variant run on Foxhound's executable, due
//! with `cli` (ADR 0094); it is the only variant that names a Product.
//!
//! The rule fails on a Journey test that names an unknown or a retired
//! Journey, a surface its Journey has no variant on, or no single variant,
//! and on two tests of one variant. Each variant of an active Journey that
//! no test carries is reported: it stays `unproved` until its test lands.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::{Journey, JourneyId, Milestone, Status, Surface};

use crate::check::Finding;

/// The variant that runs a Collie Journey's `cli` steps on Foxhound's
/// executable (ADR 0094).
pub(super) const FOXHOUND_CLI: &str = "foxhound-cli";

/// A test that names a Journey.
pub(super) struct JourneyTest {
    /// `<runner> <test ID>`, as the findings name it.
    pub(super) subject: String,
    pub(super) carried: Carried,
}

/// What a Journey test carries.
pub(super) enum Carried {
    /// One Journey's ID, as written, and one surface's name.
    Variant { journey: String, surface: String },
    /// Something other than one Journey and one surface.
    Malformed(String),
}

/// One Journey on one surface.
pub(super) struct Variant<'a> {
    pub(super) journey: &'a Journey,
    pub(super) surface: String,
    /// The milestone by which it must be proved.
    pub(super) milestone: Milestone,
}

impl Variant<'_> {
    /// `J-NNN@<surface>`.
    pub(super) fn id(&self) -> String {
        format!("{}@{}", self.journey.id, self.surface)
    }

    /// The Product whose executable it drives.
    pub(super) fn product(&self) -> Product {
        if self.surface == FOXHOUND_CLI {
            Product::Foxhound
        } else {
            self.journey.product
        }
    }
}

/// Every variant of `journey`, retired or not, in its surfaces' order, then
/// `foxhound-cli`.
pub(super) fn of(journey: &Journey) -> Vec<Variant<'_>> {
    let mut variants: Vec<Variant<'_>> = journey
        .surfaces
        .iter()
        .map(|(surface, milestone)| Variant {
            journey,
            surface: surface.clone(),
            milestone: *milestone,
        })
        .collect();
    if journey.product == Product::Collie
        && let Some(milestone) = journey.surfaces.get("cli")
    {
        variants.push(Variant {
            journey,
            surface: FOXHOUND_CLI.to_owned(),
            milestone: *milestone,
        });
    }
    variants
}

/// Whether `name` is a surface a variant can run on: one of ADR 0063's, or
/// `foxhound-cli`.
pub(crate) fn is_surface(name: &str) -> bool {
    name == FOXHOUND_CLI || Surface::from_name(name).is_some()
}

/// A variant's ID, `J-NNN@<surface>`, whatever catalog holds it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct VariantId {
    journey: JourneyId,
    surface: String,
}

impl VariantId {
    pub(crate) fn surface(&self) -> &str {
        &self.surface
    }

    pub(super) fn names(&self, variant: &Variant<'_>) -> bool {
        variant.journey.id == self.journey && variant.surface == self.surface
    }
}

impl fmt::Display for VariantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.journey, self.surface)
    }
}

impl TryFrom<String> for VariantId {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

impl FromStr for VariantId {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let (journey, surface) = text
            .split_once('@')
            .ok_or_else(|| format!("`{text}` is no variant ID, `J-NNN@<surface>`"))?;
        if !is_surface(surface) {
            return Err(format!(
                "`{surface}` is no surface of ADR 0063, nor `foxhound-cli`"
            ));
        }
        Ok(Self {
            journey: JourneyId::try_from(journey.to_owned()).map_err(|error| error.to_string())?,
            surface: surface.to_owned(),
        })
    }
}

/// The rule's findings over the catalog's Journeys, and the ID of each
/// active Journey's variant that no test carries.
pub(super) fn check(journeys: &[&Journey], tests: &[JourneyTest]) -> (Vec<Finding>, Vec<String>) {
    let by_id: BTreeMap<&str, &Journey> = journeys
        .iter()
        .map(|journey| (journey.id.as_str(), *journey))
        .collect();
    let mut findings = Vec::new();
    let mut carriers: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for test in tests {
        let subject = || test.subject.clone();
        let (journey_id, surface) = match &test.carried {
            Carried::Malformed(reason) => {
                findings.push(Finding::new("malformed-variant", subject(), reason.clone()));
                continue;
            }
            Carried::Variant { journey, surface } => (journey.as_str(), surface.as_str()),
        };
        let Some(journey) = by_id.get(journey_id) else {
            findings.push(Finding::new(
                "unknown-journey",
                subject(),
                format!("no file of `journeys/` is {journey_id}"),
            ));
            continue;
        };
        if journey.status == Status::Retired {
            findings.push(Finding::new(
                "retired-journey",
                subject(),
                format!("{journey_id} is retired, so no test runs it; test its replacement"),
            ));
            continue;
        }
        if !of(journey).iter().any(|variant| variant.surface == surface) {
            let why = if surface == FOXHOUND_CLI {
                "`foxhound-cli` is a variant only of a Collie Journey that lists `cli` (ADR 0094)"
                    .to_owned()
            } else {
                format!("{journey_id} does not list `{surface}`")
            };
            findings.push(Finding::new("unlisted-variant", subject(), why));
            continue;
        }
        carriers
            .entry(format!("{journey_id}@{surface}"))
            .or_default()
            .push(&test.subject);
    }
    for (variant, subjects) in &carriers {
        if subjects.len() > 1 {
            for subject in subjects {
                findings.push(Finding::new(
                    "duplicate-variant",
                    *subject,
                    format!(
                        "{} tests carry {variant}; keep one test per variant",
                        subjects.len()
                    ),
                ));
            }
        }
    }
    let untested = journeys
        .iter()
        .filter(|journey| journey.status == Status::Active)
        .flat_map(|journey| of(journey))
        .map(|variant| variant.id())
        .filter(|variant| !carriers.contains_key(variant))
        .collect();
    (findings, untested)
}
