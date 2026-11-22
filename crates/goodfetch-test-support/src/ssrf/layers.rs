//! The layers Collie carries the SSRF case table to (ADR 0067; spec #563's
//! story 89), as data: `test-support/fixtures/ssrf/layers.toml` lists each
//! layer, the runners that apply the table to it and the families each
//! runs, and why a layer runs no case of a family every layer meets. A
//! runner takes its cases through [`Runner`], which reads them from the
//! layer's entry, so it cannot skip a family the entry gives it, and
//! [`SsrfLayers::check`] fails a layer with no runner, a family a layer
//! skips, and a runner the entry names that its test file does not hold.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::IpAddr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Family, Miss, Outcome, SsrfCase, SsrfTable, SsrfTableError, run_cases};

/// The layers Collie carries and the runners that apply the table to each.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SsrfLayers {
    /// The families every layer meets: each layer runs their cases, or says
    /// in `not_run` why it never meets one.
    pub every_layer: Vec<Family>,
    /// The layers, by name in snake case.
    pub layers: BTreeMap<String, Layer>,
}

/// One layer a request of the run passes, which holds it to the egress
/// policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// What the layer is and how its runners reach it.
    pub summary: String,
    /// The tests that apply the table to the layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runners: Vec<LayerRunner>,
    /// Why the layer never meets a family of `every_layer`, by family.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub not_run: BTreeMap<Family, String>,
    /// The ticket that builds the layer, when it is not built yet: such a
    /// layer has no runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
}

/// A layer not built yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    /// The ticket or area that builds it, and its runner with it.
    pub joins: String,
    /// The families its runner will run.
    pub families: Vec<Family>,
}

/// One test that applies the table to a layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerRunner {
    /// The test: its file relative to the workspace's root, `::`, and its
    /// function's name.
    pub test: String,
    /// The families whose cases it runs.
    pub families: Vec<Family>,
}

impl LayerRunner {
    /// The test's file and function.
    fn file_and_function(&self) -> Option<(&str, &str)> {
        self.test.rsplit_once("::")
    }
}

/// A way the layers fall short of the table, or a runner of its test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerGap {
    /// A built layer has no runner.
    NoRunner {
        /// The layer.
        layer: String,
    },
    /// A layer neither runs a family every layer meets nor says why not.
    FamilySkipped {
        /// The layer.
        layer: String,
        /// The family.
        family: Family,
    },
    /// A layer both runs a family and says it never meets it.
    RunAndNotRun {
        /// The layer.
        layer: String,
        /// The family.
        family: Family,
    },
    /// No layer runs a family of the table, nor will a pending one.
    FamilyOfNoLayer {
        /// The family.
        family: Family,
    },
    /// A pending layer names runners.
    PendingWithRunners {
        /// The layer.
        layer: String,
    },
    /// A runner's test is not in its file, or its file does not take its
    /// cases by its name.
    RunnerNotFound {
        /// The layer.
        layer: String,
        /// The runner's test.
        test: String,
    },
}

/// Every gap of the layers, in the layers' then the families' order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerGaps {
    /// The gaps.
    pub gaps: Vec<LayerGap>,
}

/// The committed layers' text.
const COMMITTED: &str = include_str!("../../../../test-support/fixtures/ssrf/layers.toml");

impl SsrfLayers {
    /// The layers written in `text`.
    ///
    /// # Errors
    ///
    /// [`SsrfTableError::Malformed`] when the text is not TOML of the
    /// layers' format, such as a runner naming an unknown family.
    pub fn parse(text: &str) -> Result<Self, SsrfTableError> {
        Ok(toml::from_str(text)?)
    }

    /// The committed layers, `test-support/fixtures/ssrf/layers.toml`.
    ///
    /// # Errors
    ///
    /// [`SsrfTableError::Malformed`] when the committed file is, which the
    /// Quick suite catches.
    pub fn committed() -> Result<Self, SsrfTableError> {
        Self::parse(COMMITTED)
    }

    /// Checks the layers against `table` and the test files `source` reads
    /// by their path relative to the workspace's root: every built layer
    /// has a runner, runs every family of `every_layer` or says why not,
    /// every family of the table is run by a layer or will be by a pending
    /// one, and every runner's file holds its test and names it.
    ///
    /// # Errors
    ///
    /// [`LayerGaps`] listing every gap.
    pub fn check(
        &self,
        table: &SsrfTable,
        source: impl Fn(&str) -> Option<String>,
    ) -> Result<(), LayerGaps> {
        let mut gaps = Vec::new();
        let mut covered = BTreeSet::new();
        for (name, layer) in &self.layers {
            let run: BTreeSet<Family> = layer
                .runners
                .iter()
                .flat_map(|runner| runner.families.iter().copied())
                .collect();
            covered.extend(run.iter().copied());
            if let Some(pending) = &layer.pending {
                covered.extend(pending.families.iter().copied());
                if !layer.runners.is_empty() {
                    gaps.push(LayerGap::PendingWithRunners {
                        layer: name.clone(),
                    });
                }
                continue;
            }
            if layer.runners.is_empty() {
                gaps.push(LayerGap::NoRunner {
                    layer: name.clone(),
                });
            }
            for family in &self.every_layer {
                let excused = layer.not_run.contains_key(family);
                if run.contains(family) && excused {
                    gaps.push(LayerGap::RunAndNotRun {
                        layer: name.clone(),
                        family: *family,
                    });
                } else if !run.contains(family) && !excused {
                    gaps.push(LayerGap::FamilySkipped {
                        layer: name.clone(),
                        family: *family,
                    });
                }
            }
            for runner in &layer.runners {
                if !holds(
                    runner,
                    source(runner.file_and_function().map_or("", |(file, _)| file)),
                ) {
                    gaps.push(LayerGap::RunnerNotFound {
                        layer: name.clone(),
                        test: runner.test.clone(),
                    });
                }
            }
        }
        let mut families: Vec<Family> = table.cases.iter().map(|case| case.family).collect();
        families.dedup();
        let mut seen = BTreeSet::new();
        for family in families {
            if seen.insert(family) && !covered.contains(&family) {
                gaps.push(LayerGap::FamilyOfNoLayer { family });
            }
        }
        if gaps.is_empty() {
            Ok(())
        } else {
            Err(LayerGaps { gaps })
        }
    }

    /// The runner `test` of `file` applies to `layer`, with its cases of
    /// `table`: every case of each family its entry gives it, so a case the
    /// table gains runs there too, with no count of them to keep.
    ///
    /// # Errors
    ///
    /// [`RunnerError::Unknown`] when the layer's entry names no such
    /// runner, and [`RunnerError::NoCase`] when `table` has no case of a
    /// family the entry gives it, which the runner would pass running none.
    pub fn runner(
        &self,
        table: &SsrfTable,
        layer: &str,
        file: &str,
        test: &str,
    ) -> Result<Runner, RunnerError> {
        let file = file.replace('\\', "/");
        let unknown = || RunnerError::Unknown {
            layer: layer.to_owned(),
            test: format!("{file}::{test}"),
        };
        let entry = self
            .layers
            .get(layer)
            .and_then(|entry| {
                entry
                    .runners
                    .iter()
                    .find(|runner| runner.file_and_function() == Some((file.as_str(), test)))
            })
            .ok_or_else(unknown)?;
        let without: Vec<Family> = entry
            .families
            .iter()
            .copied()
            .filter(|family| table.cases_of(*family).next().is_none())
            .collect();
        if !without.is_empty() {
            return Err(RunnerError::NoCase {
                layer: layer.to_owned(),
                test: entry.test.clone(),
                families: without,
            });
        }
        let cases = entry
            .families
            .iter()
            .flat_map(|family| table.cases_of(*family))
            .cloned()
            .collect();
        Ok(Runner {
            layer: layer.to_owned(),
            test: entry.test.clone(),
            cases,
        })
    }
}

/// Whether `source`, a runner's file, holds its test and names it, as the
/// call that takes its cases does.
fn holds(runner: &LayerRunner, source: Option<String>) -> bool {
    let (Some((_, function)), Some(source)) = (runner.file_and_function(), source) else {
        return false;
    };
    source.contains(&format!("fn {function}(")) && source.contains(&format!("\"{function}\""))
}

/// Why a runner's cases cannot be taken.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RunnerError {
    /// The committed layers do not name the runner.
    #[error("the SSRF layers name no runner {test} of the layer {layer}")]
    Unknown {
        /// The layer asked for.
        layer: String,
        /// The runner's test asked for.
        test: String,
    },
    /// The table has no case of families the runner's entry gives it.
    #[error(
        "the SSRF table has no case of {families:?}, which the runner {test} of the layer {layer} runs"
    )]
    NoCase {
        /// The layer.
        layer: String,
        /// The runner's test.
        test: String,
        /// The families with no case, in the entry's order.
        families: Vec<Family>,
    },
}

/// One runner of a layer, with the cases its entry gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runner {
    /// The layer.
    pub layer: String,
    /// The runner's test.
    pub test: String,
    cases: Vec<SsrfCase>,
}

impl Runner {
    /// The committed runner `test` of `file` (the caller's `file!()`) on
    /// `layer`, its cases from the committed table.
    ///
    /// # Errors
    ///
    /// [`RunnerError::Unknown`] when the committed layers name no such
    /// runner, or a committed file is malformed, which the Quick suite
    /// catches; [`RunnerError::NoCase`] when the committed table has no
    /// case of a family the runner's entry gives it.
    pub fn committed(layer: &str, file: &str, test: &str) -> Result<Self, RunnerError> {
        let unknown = || RunnerError::Unknown {
            layer: layer.to_owned(),
            test: format!("{file}::{test}"),
        };
        let table = SsrfTable::committed().map_err(|_| unknown())?;
        let layers = SsrfLayers::committed().map_err(|_| unknown())?;
        layers.runner(&table, layer, file, test)
    }

    /// The cases the runner runs: every case of each family its entry
    /// gives it, in the table's order within each family.
    pub fn cases(&self) -> &[SsrfCase] {
        &self.cases
    }

    /// Runs `controls`, the runner's positive controls, then its cases
    /// through `layer`: each control must be let through and reach its
    /// target, and each case meet its refusal with no connection reaching
    /// its target, a target written `public` being `public`.
    ///
    /// # Errors
    ///
    /// [`RunFailed`] when the runner has no control, a control was held,
    /// or a case was let through, listing every one.
    pub fn run(
        &self,
        controls: &[SsrfCase],
        public: IpAddr,
        mut layer: impl FnMut(&SsrfCase) -> Outcome,
    ) -> Result<(), RunFailed> {
        let held: Vec<Miss> = controls
            .iter()
            .filter_map(|control| {
                let outcome = layer(control);
                let reached_target = control.target_address(public).is_some_and(|target| {
                    outcome
                        .connections
                        .iter()
                        .any(|connection| connection.to_canonical() == target.to_canonical())
                });
                (outcome.refusal.is_some() || !reached_target).then(|| Miss {
                    case: control.name.clone(),
                    expected: control.refusal.clone(),
                    refusal: outcome.refusal,
                    reached_target,
                })
            })
            .collect();
        let let_through = run_cases(&self.cases, public, &mut layer)
            .err()
            .map_or_else(Vec::new, |let_through| let_through.misses);
        if !controls.is_empty() && held.is_empty() && let_through.is_empty() {
            Ok(())
        } else {
            Err(RunFailed {
                runner: self.test.clone(),
                no_control: controls.is_empty(),
                held,
                let_through,
            })
        }
    }
}

/// What a runner's run fell short by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunFailed {
    /// The runner's test.
    pub runner: String,
    /// Whether it gave no positive control.
    pub no_control: bool,
    /// The controls held: refused, or let through to no connection.
    pub held: Vec<Miss>,
    /// The cases let through.
    pub let_through: Vec<Miss>,
}

impl fmt::Display for RunFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the SSRF runner {} failed:", self.runner)?;
        if self.no_control {
            formatter.write_str("\n  it gave no positive control")?;
        }
        for miss in &self.held {
            write!(
                formatter,
                "\n  the control {} was held: {:?}, reached its target: {}",
                miss.case, miss.refusal, miss.reached_target
            )?;
        }
        for miss in &self.let_through {
            write!(formatter, "\n  {miss}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RunFailed {}

impl fmt::Display for LayerGap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRunner { layer } => write!(formatter, "the layer {layer} has no runner"),
            Self::FamilySkipped { layer, family } => write!(
                formatter,
                "the layer {layer} runs no {family:?} case and says not why"
            ),
            Self::RunAndNotRun { layer, family } => write!(
                formatter,
                "the layer {layer} runs {family:?} cases and says it never meets them"
            ),
            Self::FamilyOfNoLayer { family } => {
                write!(formatter, "no layer runs the {family:?} cases")
            }
            Self::PendingWithRunners { layer } => {
                write!(formatter, "the pending layer {layer} names runners")
            }
            Self::RunnerNotFound { layer, test } => write!(
                formatter,
                "the layer {layer}'s runner {test} is not in its file, or does not take its cases by its name"
            ),
        }
    }
}

impl fmt::Display for LayerGaps {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the SSRF layers fall short:")?;
        for gap in &self.gaps {
            write!(formatter, "\n  {gap}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LayerGaps {}

#[cfg(test)]
mod tests;
