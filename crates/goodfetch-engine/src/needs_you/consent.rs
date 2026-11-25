//! The consent walls of "Needs you" (ADRs 0048 and 0052; spec #591's story
//! 26), an engine source its contribution passes in: each domain whose
//! consent wall a Run met and left, as the Run index keeps them, offered
//! for `consent.domains` until the machine's own layers, the Operator
//! policy and user configuration, cover it. A domain in
//! `wall.denied_domains` is offered too: a denial takes away the Wall
//! allowance, never the approval to accept a consent wall. A person may
//! dismiss an item. It reads again at each storage sample, which also
//! catches another engine's Runs and an edited configuration file.

use goodfetch_protocol::needs_you::{CONSENT_WALL, NEEDS_YOU_ALLOW_CONSENT_DOMAIN, NeedsYouItem};
use goodfetch_protocol::settings::CONSENT_DOMAINS;
use serde_json::{Map, Value, json};

use super::{Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError};
use crate::host_name;
use crate::run_index::ConsentWallSeen;
use crate::settings::Opening;

/// Each domain whose consent wall a Run met, which `consent.domains` does
/// not cover yet.
pub fn walls() -> NeedsYouSource {
    NeedsYouSource::new(Walls)
}

/// `consent.domains` as resolved: each entry keyed as a host is everywhere
/// (`host_name`), so `Bücher.de.` and `xn--bcher-kva.de` are one domain, or
/// `*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentDomains(Vec<String>);

impl ConsentDomains {
    /// The domains `value`, the setting's value, lists.
    pub fn of(value: &Value) -> Self {
        Self(
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(host_name::lookup_key)
                .collect(),
        )
    }

    /// Whether it lists nothing, which approves nothing.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether it covers `domain`, a domain's key: it lists it, or `*`.
    pub fn covers(&self, domain: &str) -> bool {
        self.0
            .iter()
            .any(|listed| listed == "*" || listed == domain)
    }
}

/// What a person takes on by adding `domain`, or `*`, to `consent.domains`
/// (ADRs 0048 and 0052), in the words a person surface shows beside the
/// answer and the Audit log records with it.
pub fn warning(domain: &str) -> String {
    if domain == "*" {
        "Accepting consent walls on every domain gives consent in your name on every site that shows one: those sites may load their trackers and set their cookies, and under the GDPR that consent is yours. An Agent also no longer asks you before accepting a consent prompt on any domain. Every later run accepts them until you remove `*` from `consent.domains`.".to_owned()
    } else {
        format!(
            "Accepting the consent walls of {domain} gives consent in your name: the site may load its trackers and set their cookies, and under the GDPR that consent is yours. Every later run accepts them until you remove {domain} from `consent.domains`."
        )
    }
}

struct Walls;

impl Primitives for Walls {
    fn dismissable(&self) -> bool {
        true
    }

    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        Box::pin(async move {
            let seen = context
                .run_index
                .consent_walls()
                .await
                .map_err(SourceError::new)?;
            if seen.is_empty() {
                return Ok(Vec::new());
            }
            let (machine, _) = context
                .settings
                .open(Opening::default())
                .await
                .map_err(SourceError::new)?;
            let domains = ConsentDomains::of(machine.resolved().value(&CONSENT_DOMAINS));
            Ok(seen
                .iter()
                .filter(|wall| !domains.covers(&wall.domain))
                .map(item)
                .collect())
        })
    }

    fn changes(&self, context: &SourceContext) -> Changes {
        Changes::of(context.storage())
    }
}

/// The item of the domain `wall` names.
fn item(wall: &ConsentWallSeen) -> NeedsYouItem {
    let domain = &wall.domain;
    let mut details = Map::new();
    details.insert("domain".to_owned(), json!(domain));
    NeedsYouItem {
        id: format!("{CONSENT_WALL}:{domain}"),
        kind: CONSENT_WALL.to_owned(),
        since: wall.first_seen,
        run: Some(wall.last_run.clone()),
        schedule: None,
        summary: format!(
            "{domain} showed a consent wall, which no run accepts unless a person approved it: its pages stayed at `consent_required`. Add {domain} to `consent.domains` for the next runs to accept its consent walls, which gives consent in your name."
        ),
        actions: vec![NEEDS_YOU_ALLOW_CONSENT_DOMAIN.name.to_owned()],
        details,
    }
}
