//! A crawl declared as data (spec #611's #1514; ADR 0103): the start's
//! `declaration`, checked and merged into the start as the Labels,
//! Datasets, settings and seeds it declares, then planned, Label by Label,
//! as what each of its pages follows and extracts. Its patterns compile
//! once, as the Run starts (D14).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_extractor::fields::Fields;
use goodfetch_frontier::Patterns;
use goodfetch_protocol::foxhound::crawl::{
    CrawlStartParams, ExtractionFields, LABEL_UNKNOWN, LabelDeclaration, LabelUnknownDetails, Seeds,
};
use goodfetch_protocol::foxhound::declaration::{CrawlDeclaration, DeclaredLabel, FollowRule};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};
use serde_json::{Map, Value};

use super::Demand;

/// What the pages of one declared Label do: the links each follows, rule
/// by rule, and the Records each pushes, one per extract, of the fields
/// read once for them all.
pub(crate) struct Plan {
    /// Its follow rules, in order.
    pub(super) follow: Vec<FollowRule>,
    /// Every field its extracts read, in the order declared.
    pub(super) fields: ExtractionFields,
    /// The reader of [`Self::fields`], each `match` pattern and locale's
    /// forms learned once; none when the crawl keeps no Record.
    pub(super) reader: Option<Arc<Fields>>,
    /// Each extract's Dataset and the names of its fields, in order.
    pub(super) extracts: Vec<(String, Vec<String>)>,
}

impl Plan {
    /// What the readers of its pages' untouched trees read for it: the
    /// selectors of its follow rules, each once, and its fields' locators
    /// when every one is a value field.
    pub(super) fn demand(&self) -> Demand {
        let mut seen = BTreeSet::new();
        let links = self
            .follow
            .iter()
            .filter(|rule| seen.insert(rule.links.as_str()))
            .map(|rule| rule.links.clone())
            .collect();
        (links, super::locators(self.fields.by_name()))
    }
}

/// Each declared Label's [`Plan`], by name.
pub(crate) type Plans = BTreeMap<String, Plan>;

/// Merges the start `params`' `declaration`, if any, into it, checked on
/// `work`, the engine's Counted work: its Labels and Datasets become the
/// start's, its settings lie beneath the start's, which win key by key,
/// and its seeds follow the start's.
///
/// # Errors
///
/// `setting_invalid` for a start naming `extraction` too; `label_unknown`
/// for a follow rule naming a Label the declaration does not declare;
/// `invalid_params` for a start declaring a Label or a Dataset of its own,
/// a Label declared twice, an extract into a Dataset the declaration does
/// not declare, a field name repeated across one Label's extracts, or a
/// selector, pattern or field the engine could never read.
pub(super) fn merge(params: &mut CrawlStartParams, work: &CountedWork) -> Result<(), CallError> {
    let Some(declaration) = &params.declaration else {
        return Ok(());
    };
    if params.extraction.is_some() {
        return Err(CallError::new(
            &SETTING_INVALID,
            "A crawl start names both `extraction` and `declaration`, so it was not started: `extraction` is a declaration's one-Label shorthand.",
            &SettingInvalidDetails {
                key: "declaration".to_owned(),
                layer: None,
                reason: "`extraction` and `declaration` both declare how the engine handles each request".to_owned(),
                limit: None,
                fix: Some("Leave out `extraction`, or `declaration`.".to_owned()),
            },
        ));
    }
    if let Some(label) = params.labels.keys().next() {
        return Err(invalid(format!(
            "A crawl with `declaration` takes its Labels from it, so it takes none of the start's, such as `{label}`: declare it in the declaration's `labels`."
        )));
    }
    if let Some(dataset) = params.datasets.first() {
        return Err(invalid(format!(
            "A crawl with `declaration` takes its Datasets from it, so it takes none of the start's, such as `{}`: declare it in the declaration's `datasets`.",
            dataset.name
        )));
    }
    check(declaration, work)?;
    let declaration = declaration.clone();
    params
        .labels
        .extend(declaration.labels.into_iter().map(|label| {
            (
                label.name,
                LabelDeclaration {
                    payload: label.payload_schema,
                    settings: label.settings,
                },
            )
        }));
    params.datasets.extend(declaration.datasets);
    beneath(&declaration.settings, &mut params.settings);
    params.seeds = Seeds::new(
        params
            .seeds
            .iter()
            .cloned()
            .chain(declaration.seeds)
            .collect(),
    )
    .expect("invariant: a start's seeds are never empty");
    Ok(())
}

/// Refuses `declaration` unless the engine can run it as declared, its
/// selectors, patterns and fields compiled on `work`.
fn check(declaration: &CrawlDeclaration, work: &CountedWork) -> Result<(), CallError> {
    let mut names = BTreeSet::new();
    if let Some(twice) = declaration
        .labels
        .iter()
        .find(|label| !names.insert(label.name.as_str()))
    {
        return Err(invalid(format!(
            "The declaration declares the Label `{}` twice: give each of its Labels a name of its own.",
            twice.name
        )));
    }
    let datasets: BTreeSet<&str> = declaration
        .datasets
        .iter()
        .map(|dataset| dataset.name.as_str())
        .collect();
    for label in &declaration.labels {
        for rule in &label.follow {
            followed(rule, &names, work)?;
        }
        extracted(label, &datasets, work)?;
    }
    Ok(())
}

/// Refuses `rule` unless it follows into a Label of `declared` with a
/// selector and patterns the engine reads, compiled on `work`.
fn followed(
    rule: &FollowRule,
    declared: &BTreeSet<&str>,
    work: &CountedWork,
) -> Result<(), CallError> {
    if !declared.contains(rule.label.as_str()) {
        let declared: Vec<String> = declared.iter().map(|&name| name.to_owned()).collect();
        return Err(CallError::new(
            &LABEL_UNKNOWN,
            format!(
                "The declaration declares no Label `{}`, so no rule follows links into it: declare it in `labels`, or follow into one it declares ({}).",
                rule.label,
                declared.join(", ")
            ),
            &LabelUnknownDetails {
                label: rule.label.clone(),
                declared,
            },
        ));
    }
    goodfetch_fetch_tier::check_selector(&rule.links, work)
        .map_err(|refused| invalid(refused.to_string()))?;
    for globs in [&rule.include, &rule.exclude] {
        Patterns::cached(work, Row::BatchCompilations, globs).map_err(CallError::from)?;
    }
    Ok(())
}

/// Refuses `label`'s extracts unless each pushes into a Dataset of
/// `datasets` fields the engine reads, checked on `work`, and no field
/// name repeats across them.
fn extracted(
    label: &DeclaredLabel,
    datasets: &BTreeSet<&str>,
    work: &CountedWork,
) -> Result<(), CallError> {
    let mut names = BTreeSet::new();
    for extract in &label.extract {
        if !datasets.contains(extract.dataset.as_str()) {
            return Err(invalid(format!(
                "The Label `{}` extracts into `{}`, a Dataset the declaration does not declare: declare it in `datasets`.",
                label.name, extract.dataset
            )));
        }
        goodfetch_fetch_tier::check_fields(extract.fields.by_name(), work).map_err(invalid)?;
        if let Some(twice) = extract
            .fields
            .declared()
            .iter()
            .find(|name| !names.insert(name.as_str()))
        {
            return Err(invalid(format!(
                "The Label `{}` reads the field `{twice}` in two extracts: name each field of a Label's extracts once.",
                label.name
            )));
        }
    }
    Ok(())
}

/// Each Label of `declaration`, planned: its fields' reader built on
/// `work` unless the crawl `keeps_records` not.
pub(super) fn plans(
    declaration: CrawlDeclaration,
    keeps_records: bool,
    work: &CountedWork,
) -> Plans {
    declaration
        .labels
        .into_iter()
        .map(|label| {
            let fields: ExtractionFields =
                label
                    .extract
                    .iter()
                    .flat_map(|extract| {
                        let by_name = extract.fields.by_name();
                        extract.fields.declared().iter().filter_map(move |name| {
                            Some((name.clone(), by_name.get(name)?.clone()))
                        })
                    })
                    .collect();
            let plan = Plan {
                reader: (keeps_records && !fields.is_empty())
                    .then(|| Arc::new(Fields::of(fields.by_name(), work))),
                extracts: label
                    .extract
                    .into_iter()
                    .map(|extract| (extract.dataset, extract.fields.declared().to_vec()))
                    .collect(),
                follow: label.follow,
                fields,
            };
            (label.name, plan)
        })
        .collect()
}

/// `lower`'s settings beneath `upper`'s: each key `upper` lacks is
/// `lower`'s, and nested maps merge, `upper` winning key by key.
fn beneath(lower: &Map<String, Value>, upper: &mut Map<String, Value>) {
    for (key, value) in lower {
        match (upper.get_mut(key), value) {
            (Some(Value::Object(upper)), Value::Object(lower)) => beneath(lower, upper),
            (Some(_), _) => {}
            (None, _) => {
                upper.insert(key.clone(), value.clone());
            }
        }
    }
}

/// An `invalid_params` refusal saying `message`.
fn invalid(message: String) -> CallError {
    CallError::protocol(ProtocolFailure::InvalidParams, message)
}
