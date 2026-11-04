//! Add Seeds in the details window: offered for a live crawl on a Foxhound
//! Daemon while `controls` is on, it asks a confirmation naming the Run,
//! how many seeds, their Label and the consequence before it adds
//! anything, then says what the crawl did with them.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{RequestSeed, Seed};
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::run::{RunId, RunKind};

use super::super::attempt;
use super::{Details, NAME_MOST, VALUE_MOST, counted, name};
use crate::client::{Call, CallFailure, SeedsAdded, SeedsToAdd};
use crate::text;

/// Add Seeds, offered for a live crawl on a Foxhound Daemon while
/// `controls` is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedsOffer {
    pub(super) run_id: RunId,
    /// The crawl's name, shown.
    pub(super) name: String,
}

impl SeedsOffer {
    /// The question Add Seeds asks before it adds the seeds `urls`, one a
    /// line, with the Label `label`: naming the Run and the consequence;
    /// or what the person has yet to give.
    ///
    /// # Errors
    ///
    /// The Label is blank, or no line holds a URL.
    pub fn confirmation(&self, label: &str, urls: &str) -> Result<SeedsConfirmation, String> {
        let label = label.trim();
        if label.is_empty() {
            return Err("Give the Label of the handler the seeds go to".to_owned());
        }
        let seeds: Vec<Seed> = urls
            .lines()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(|url| {
                Seed::Request(RequestSeed {
                    url: url.to_owned(),
                    label: label.to_owned(),
                    payload: None,
                    priority: 0,
                    forefront: false,
                    method: HttpMethod::default(),
                    body: None,
                    key: None,
                })
            })
            .collect();
        if seeds.is_empty() {
            return Err("Give at least one URL, one a line".to_owned());
        }
        let described = if self.name == self.run_id.as_str() {
            self.name.clone()
        } else {
            format!("{} ({})", self.name, self.run_id)
        };
        Ok(SeedsConfirmation {
            title: text::shown(
                &format!("Add {} to {}?", counted(seeds.len(), "seed"), self.name),
                VALUE_MOST,
            ),
            consequence: text::shown(
                &format!(
                    "The crawl {described} fetches them with the Label {label}, beside its other requests. A seed it already holds is not added again, and adding cannot be undone."
                ),
                VALUE_MOST * 2,
            ),
            confirm: "Add seeds".to_owned(),
            run_id: self.run_id.clone(),
            seeds,
        })
    }
}

/// The question Add Seeds asks: the person's yes, given to
/// [`Details::confirmed`], adds the seeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedsConfirmation {
    /// The question, naming the Run and how many seeds.
    pub title: String,
    /// What adding does to the crawl.
    pub consequence: String,
    /// The label of the button that confirms.
    pub confirm: String,
    run_id: RunId,
    seeds: Vec<Seed>,
}

/// Where an Add Seeds is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Seeding {
    /// Its seeds are being added.
    Adding(usize),
    /// The crawl answered.
    Added(SeedsAdded),
    /// It failed.
    Failed(CallFailure),
}

impl Details {
    /// Add Seeds for the Run `id`, when it offers it: a live crawl, on a
    /// Foxhound Daemon, while the settings, once read, leave `controls`
    /// on.
    pub(super) fn seeds_offer(&self, id: &RunId) -> Option<SeedsOffer> {
        let controls = self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.controls);
        let row = self.row(id)?;
        let offered = controls
            && self.product() == Some(Product::Foxhound)
            && row.kind == RunKind::Crawl
            && self.runs(id);
        offered.then(|| SeedsOffer {
            run_id: id.clone(),
            name: text::shown(&name(row), NAME_MOST),
        })
    }

    /// The person said yes to `confirmation`: the call that adds its
    /// seeds, unless the Run no longer offers Add Seeds.
    pub fn confirmed(&mut self, confirmation: SeedsConfirmation) -> Option<Call> {
        let SeedsConfirmation { run_id, seeds, .. } = confirmation;
        self.seeds_offer(&run_id)?;
        self.seeds = Some((run_id.clone(), Seeding::Adding(seeds.len())));
        Some(Call::AddSeeds(SeedsToAdd { run_id, seeds }))
    }

    /// What the window says of an Add Seeds of the Run `id`.
    pub(super) fn seeding(&self, id: &RunId, seeding: &Seeding) -> String {
        let said = match seeding {
            Seeding::Adding(count) => format!("Adding {}…", counted(*count, "seed")),
            Seeding::Added(added) => {
                let count = usize::try_from(added.added).unwrap_or(usize::MAX);
                let not_added: u64 = added.not_added.values().sum();
                if not_added == 0 {
                    format!("{} added", counted(count, "seed"))
                } else {
                    let reasons: Vec<String> = added
                        .not_added
                        .iter()
                        .map(|(reason, count)| format!("{reason} {count}"))
                        .collect();
                    format!(
                        "{} added; {not_added} not added: {}",
                        counted(count, "seed"),
                        reasons.join(", ")
                    )
                }
            }
            Seeding::Failed(failure) => {
                let call = Call::AddSeeds(SeedsToAdd {
                    run_id: id.clone(),
                    seeds: Vec::new(),
                });
                let tried = attempt(&call, |run| {
                    self.row(run).map_or_else(|| run.as_str().to_owned(), name)
                });
                format!("{tried} failed: {} ({})", failure.message, failure.code)
            }
        };
        text::shown(&said, VALUE_MOST)
    }
}
