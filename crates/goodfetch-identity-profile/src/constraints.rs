//! The kinds of browser a profile is drawn among: an engine family's
//! population, narrowed by the `stealth.identity.*` settings.

use std::collections::BTreeSet;
use std::fmt;

use crate::candidates::all;
use crate::population::{Brand, Device, EngineFamily, Kind, Os};

/// An engine family's population narrowed to some operating systems,
/// devices and brands, each a value the population holds and none
/// leaving it empty. Only [`Constraints::narrowed`] makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraints {
    family: EngineFamily,
    os: Vec<Os>,
    device: Vec<Device>,
    brand: Vec<Brand>,
}

/// A `stealth.identity.*` setting's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NarrowingKey {
    Os,
    Device,
    Brand,
}

impl fmt::Display for NarrowingKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Os => "os",
            Self::Device => "device",
            Self::Brand => "brand",
        })
    }
}

/// Why a narrowing draws from nothing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NarrowingError {
    #[error(
        "`{value}` is no {key} of the {family} population, which holds {}",
        .population.join(", ")
    )]
    OutsidePopulation {
        family: EngineFamily,
        key: NarrowingKey,
        value: String,
        population: Vec<&'static str>,
    },
    #[error("no browser of the {family} population has this {key} and the identity's other values")]
    Empty {
        family: EngineFamily,
        key: NarrowingKey,
    },
}

impl NarrowingError {
    /// The setting whose values the narrowing stopped at.
    pub fn key(&self) -> NarrowingKey {
        match self {
            Self::OutsidePopulation { key, .. } | Self::Empty { key, .. } => *key,
        }
    }
}

impl Constraints {
    /// `family`'s population narrowed to the operating systems, devices
    /// and brands named, by their setting words; an empty list keeps every
    /// value. Narrows by operating system, then device, then brand, and
    /// names the first key whose values the population does not hold or
    /// that leaves it empty.
    pub fn narrowed(
        family: EngineFamily,
        os: &[String],
        device: &[String],
        brand: &[String],
    ) -> Result<Self, NarrowingError> {
        let whole: Vec<Kind> = all()
            .iter()
            .map(|candidate| candidate.kind)
            .filter(|kind| kind.brand.family() == family)
            .collect();
        let mut kept = whole.clone();
        Ok(Self {
            family,
            os: narrow(family, os, &whole, &mut kept)?,
            device: narrow(family, device, &whole, &mut kept)?,
            brand: narrow(family, brand, &whole, &mut kept)?,
        })
    }

    /// Whether a browser of `kind` may be drawn.
    pub(crate) fn admits(&self, kind: Kind) -> bool {
        kind.brand.family() == self.family
            && self.os.contains(&kind.os)
            && self.device.contains(&kind.device)
            && self.brand.contains(&kind.brand)
    }
}

/// One `stealth.identity.*` setting's values.
trait Facet: Copy + Ord {
    const KEY: NarrowingKey;
    fn of(kind: Kind) -> Self;
    fn word(self) -> &'static str;
}

impl Facet for Os {
    const KEY: NarrowingKey = NarrowingKey::Os;
    fn of(kind: Kind) -> Self {
        kind.os
    }
    fn word(self) -> &'static str {
        self.as_str()
    }
}

impl Facet for Device {
    const KEY: NarrowingKey = NarrowingKey::Device;
    fn of(kind: Kind) -> Self {
        kind.device
    }
    fn word(self) -> &'static str {
        self.as_str()
    }
}

impl Facet for Brand {
    const KEY: NarrowingKey = NarrowingKey::Brand;
    fn of(kind: Kind) -> Self {
        kind.brand
    }
    fn word(self) -> &'static str {
        self.as_str()
    }
}

/// The values of `F` named by `requested`, or all `whole` holds when it
/// names none, with `kept` narrowed to them.
fn narrow<F: Facet>(
    family: EngineFamily,
    requested: &[String],
    whole: &[Kind],
    kept: &mut Vec<Kind>,
) -> Result<Vec<F>, NarrowingError> {
    let population: BTreeSet<F> = whole.iter().map(|&kind| F::of(kind)).collect();
    let chosen = if requested.is_empty() {
        population.into_iter().collect()
    } else {
        requested
            .iter()
            .map(|value| {
                population
                    .iter()
                    .copied()
                    .find(|facet| facet.word() == value)
                    .ok_or_else(|| NarrowingError::OutsidePopulation {
                        family,
                        key: F::KEY,
                        value: value.clone(),
                        population: population.iter().map(|facet| facet.word()).collect(),
                    })
            })
            .collect::<Result<Vec<F>, _>>()?
    };
    kept.retain(|&kind| chosen.contains(&F::of(kind)));
    if kept.is_empty() {
        return Err(NarrowingError::Empty {
            family,
            key: F::KEY,
        });
    }
    Ok(chosen)
}
