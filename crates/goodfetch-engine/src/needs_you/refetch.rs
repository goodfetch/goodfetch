//! The stops of the refetch without the site's stored data in "Needs you"
//! (ADR 0052; spec #591's story 27), an engine source its contribution
//! passes in: each domain on which a Run's refetch stopped, as the Run
//! index keeps it, listed with the layer of `wall.refetch_domains` that
//! lists it and that layer's file, until no layer lists the domain or
//! `wall.denied_domains` denies it. The layer is the Run's project
//! configuration when that set the key and still does, otherwise the
//! machine's: user configuration when it sets the key, else the Operator
//! policy. A person may dismiss an item, remove the domain from a machine
//! layer (`needs_you.remove_refetch_domain`, in [`crate::needs_you_operations`])
//! or deny it; a domain from project configuration is shown with its file
//! and offers no removal. It reads again at each storage sample, which
//! also catches another engine's Runs and an edited file.

use std::path::Path;

use goodfetch_protocol::needs_you::{
    NEEDS_YOU_DENY_WALL_DOMAIN, NEEDS_YOU_REMOVE_REFETCH_DOMAIN, NeedsYouItem, WALL_REFETCH_STOPPED,
};
use goodfetch_protocol::settings::{SettingsLayer, WALL_DENIED_DOMAINS, WALL_REFETCH_DOMAINS};
use serde_json::{Map, Value, json};

use super::{Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError};
use crate::host_name::lookup_key;
use crate::run_index::{self, RefetchStopped};
use crate::settings::{MachineLayers, Opening, nested, project_settings};

/// Each domain on which a Run's refetch stopped that a layer still lists.
pub fn stops() -> NeedsYouSource {
    NeedsYouSource::new(Stops)
}

/// The layer of `wall.refetch_domains` a machine's runs take, and what it
/// lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listing {
    /// User configuration or the Operator policy.
    pub(crate) layer: SettingsLayer,
    /// The table of its file that sets the key: `settings`, or an Operator
    /// policy's `lock`.
    pub(crate) table: &'static str,
    /// The domains it lists, as written.
    pub(crate) domains: Vec<String>,
}

/// The machine's layer of `wall.refetch_domains`: user configuration when
/// it sets the key, else the Operator policy's lock, else its settings.
pub(crate) fn machine_listing(machine: &MachineLayers) -> Option<Listing> {
    let key = WALL_REFETCH_DOMAINS.name;
    let policy = &machine.operator_policy;
    let (layer, table, value) = if let Some(value) = machine.user_configuration.get(key) {
        (SettingsLayer::UserConfiguration, "settings", value)
    } else if let Some(value) = policy.lock.get(key) {
        (SettingsLayer::OperatorPolicy, "lock", value)
    } else {
        (
            SettingsLayer::OperatorPolicy,
            "settings",
            policy.settings.get(key)?,
        )
    };
    Some(Listing {
        layer,
        table,
        domains: domains_of(value),
    })
}

/// The text entries of a list value.
fn domains_of(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|domains| {
            domains
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// How `domains` covers `domain`: `Some(false)` when it lists it,
/// `Some(true)` when only `*` covers it, `None` when it does not.
pub(crate) fn coverage(domains: &[String], domain: &str) -> Option<bool> {
    if domains.iter().any(|listed| lookup_key(listed) == domain) {
        Some(false)
    } else if domains.iter().any(|listed| listed == "*") {
        Some(true)
    } else {
        None
    }
}

struct Stops;

impl Primitives for Stops {
    fn dismissable(&self) -> bool {
        true
    }

    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let path = context.run_index.clone();
        let version = context.version.clone();
        Box::pin(async move {
            let stops = context
                .blocking(move || run_index::refetch_stops(&path, &version))
                .await
                .map_err(SourceError::new)?;
            if stops.is_empty() {
                return Ok(Vec::new());
            }
            let (resolved, machine) = context
                .settings
                .open(Opening::default())
                .await
                .map_err(SourceError::new)?;
            let denied = domains_of(resolved.resolved().value(&WALL_DENIED_DOMAINS));
            let files = context.settings.files().clone();
            let projects: Vec<(Option<String>, String)> = stops
                .iter()
                .map(|stop| (stop.project_file.clone(), stop.domain.clone()))
                .collect();
            let projects = context
                .blocking(move || {
                    projects
                        .into_iter()
                        .map(|(file, domain)| file.and_then(|file| project_listing(&file, &domain)))
                        .collect::<Vec<_>>()
                })
                .await;
            let machine = machine_listing(&machine);
            Ok(stops
                .iter()
                .zip(projects)
                .filter(|(stop, _)| {
                    !denied
                        .iter()
                        .any(|listed| lookup_key(listed) == stop.domain)
                })
                .filter_map(|(stop, project)| {
                    let (layer, file, domains) = match (project, &machine) {
                        (Some((file, domains)), _) => {
                            (SettingsLayer::ProjectConfiguration, file, domains)
                        }
                        (None, Some(listing)) => (
                            listing.layer,
                            match listing.layer {
                                SettingsLayer::UserConfiguration => files.user_configuration(),
                                _ => files.operator_policy(),
                            }
                            .display()
                            .to_string(),
                            listing.domains.clone(),
                        ),
                        (None, None) => return None,
                    };
                    let only_star = coverage(&domains, &stop.domain)?;
                    Some(item(stop, layer, &file, only_star))
                })
                .collect())
        })
    }

    fn changes(&self, context: &SourceContext) -> Changes {
        Changes::of(context.storage())
    }
}

/// The file at `file` and what its `wall.refetch_domains` lists, when it
/// still sets the key; a file that cannot be read is taken to list the
/// domain still, as it did when the Run stopped. Blocking.
fn project_listing(file: &str, domain: &str) -> Option<(String, Vec<String>)> {
    match project_settings(Path::new(file)) {
        Ok(Some(settings)) => nested(&settings, WALL_REFETCH_DOMAINS.name)
            .map(|value| (file.to_owned(), domains_of(value))),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, "a project configuration a stopped refetch came from cannot be read: its domain stays listed");
            Some((file.to_owned(), vec![domain.to_owned()]))
        }
    }
}

/// The item of the stop `stop`, its domain listed in `layer`'s `file`.
fn item(stop: &RefetchStopped, layer: SettingsLayer, file: &str, only_star: bool) -> NeedsYouItem {
    let domain = &stop.domain;
    let mut details = Map::new();
    details.insert("domain".to_owned(), json!(domain));
    details.insert("cause".to_owned(), json!(stop.cause));
    details.insert("layer".to_owned(), json!(layer));
    details.insert("file".to_owned(), json!(file));
    details.insert("only_star".to_owned(), json!(only_star));
    let mut actions = Vec::new();
    if layer != SettingsLayer::ProjectConfiguration {
        actions.push(NEEDS_YOU_REMOVE_REFETCH_DOMAIN.name.to_owned());
    }
    actions.push(NEEDS_YOU_DENY_WALL_DOMAIN.name.to_owned());
    let listed = if only_star {
        format!("`*` in {} covers it", layer.as_str())
    } else {
        format!("{} lists it", layer.as_str())
    };
    NeedsYouItem {
        id: format!("{WALL_REFETCH_STOPPED}:{domain}"),
        kind: WALL_REFETCH_STOPPED.to_owned(),
        since: stop.first_stopped.clone(),
        run: Some(stop.last_run.clone()),
        schedule: None,
        summary: format!(
            "{domain} refused a page fetched again without its stored cookies and storage ({}), so no new fetch of that kind starts there for the rest of the run. `wall.refetch_domains` still allows it for the next runs: {listed}, in `{file}`. Remove it there, or deny {domain} in `wall.denied_domains` with a reason.",
            stop.cause
        ),
        actions,
        details,
    }
}
