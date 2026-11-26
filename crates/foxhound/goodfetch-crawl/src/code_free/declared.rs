//! A crawl declared as data (spec #611's #1514; ADR 0103): the start's
//! `declaration`, checked and merged into the start as the Labels,
//! Datasets, settings and seeds it declares, then read, Label by Label,
//! as what each of its pages follows and extracts. Its patterns compile
//! once, as the Run starts or resumes (D14).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::CallError;
use goodfetch_engine::settings::{SettingsError, beneath_connection, check_recipe_may_set};
use goodfetch_extractor::fields::Fields;
use goodfetch_frontier::Patterns;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::crawl::{
    CrawlStartParams, ExtractionFields, LABEL_UNKNOWN, LabelDeclaration, LabelUnknownDetails, Seeds,
};
use goodfetch_protocol::foxhound::declaration::{CrawlDeclaration, DeclaredLabel, FollowRule};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::read::PageField;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};

use super::Demand;

/// What the pages of one declared Label do: the links each follows, rule
/// by rule, and the Records each pushes, one per extract, of the fields
/// read once for them all.
pub(crate) struct LabelHandling {
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

impl LabelHandling {
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

/// Each declared Label's [`LabelHandling`], by name.
pub(crate) type LabelHandlings = BTreeMap<String, LabelHandling>;

/// Merges the start `params`' `declaration`, if any, into it, checked on
/// `work`, the engine's Counted work, for an engine of `engine`: its
/// Labels and Datasets become the start's, its settings lie beneath the
/// start's, which win key by key, each key's value whole, and its seeds
/// follow the start's, of which there is still at least one. `refused`
/// answers a refused setting.
///
/// # Errors
///
/// `setting_invalid` for a start naming `extraction` too; a settings
/// refusal for a declaration setting a key a Recipe may not set, such as
/// `proxy.server`; `label_unknown` for a follow rule naming a Label the
/// declaration does not declare; `invalid_params` for a start declaring a
/// Label or a Dataset of its own, a Label declared twice, an extract into
/// a Dataset the declaration does not declare, one field name read two
/// ways across one Label's extracts, or a selector, pattern or field the
/// engine could never read.
pub(super) fn merge(
    params: &mut CrawlStartParams,
    engine: Product,
    work: &CountedWork,
    refused: &dyn Fn(&SettingsError) -> CallError,
) -> Result<(), CallError> {
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
    check_recipe_may_set(&declaration.settings, engine).map_err(|error| refused(&error))?;
    params.settings = beneath_connection(&declaration.settings, &params.settings, engine)
        .map_err(|error| refused(&error))?;
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
    compiled(rule, work)
}

/// Compiles `rule`'s patterns on `work`, from the cache it keeps for the
/// Run; `invalid_params` for a pattern the engine does not support.
fn compiled(rule: &FollowRule, work: &CountedWork) -> Result<(), CallError> {
    for globs in [&rule.include, &rule.exclude] {
        Patterns::cached(work, Row::BatchCompilations, globs).map_err(CallError::from)?;
    }
    Ok(())
}

/// Refuses `label`'s extracts unless each pushes into a Dataset of
/// `datasets` fields the engine reads, checked on `work`, and no field
/// name is read two ways across them.
fn extracted(
    label: &DeclaredLabel,
    datasets: &BTreeSet<&str>,
    work: &CountedWork,
) -> Result<(), CallError> {
    let mut read: BTreeMap<&str, &PageField> = BTreeMap::new();
    for extract in &label.extract {
        if !datasets.contains(extract.dataset.as_str()) {
            return Err(invalid(format!(
                "The Label `{}` extracts into `{}`, a Dataset the declaration does not declare: declare it in `datasets`.",
                label.name, extract.dataset
            )));
        }
        goodfetch_fetch_tier::check_fields(extract.fields.by_name(), work).map_err(invalid)?;
        for (name, field) in extract.fields.by_name() {
            if read
                .insert(name, field)
                .is_some_and(|before| before != field)
            {
                return Err(invalid(format!(
                    "The Label `{}` reads the field `{name}` two ways in its extracts: read it the same way in each, or give one of them another name.",
                    label.name
                )));
            }
        }
    }
    Ok(())
}

/// Each Label of `declaration`, as its pages are handled: its patterns
/// compiled on `work`, which keeps them for the Run, and its fields'
/// reader built on `work` unless the crawl `keeps_records` not.
pub(super) fn handlings(
    declaration: CrawlDeclaration,
    keeps_records: bool,
    work: &CountedWork,
) -> LabelHandlings {
    declaration
        .labels
        .into_iter()
        .map(|label| {
            for rule in &label.follow {
                if let Err(error) = compiled(rule, work) {
                    tracing::warn!(
                        %error,
                        label = %label.name,
                        "a declared follow rule's pattern no longer compiles"
                    );
                }
            }
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
            let handling = LabelHandling {
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
            (label.name, handling)
        })
        .collect()
}

/// An `invalid_params` refusal saying `message`.
fn invalid(message: String) -> CallError {
    CallError::protocol(ProtocolFailure::InvalidParams, message)
}
