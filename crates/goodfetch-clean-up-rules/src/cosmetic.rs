//! The cosmetic rules of the Filter lists that are on, for one page (ADR
//! 0047): live hiding and extraction-side removal both read them. Plain
//! CSS selectors come from the page's URL (its host's own rules and the
//! generic rules no class or ID keys) and from the classes and IDs the page
//! holds (the generic rules those key), as a blocker queries them; the
//! procedural rules (`:has-text`, `:upward` and the like) come apart, since
//! only some translate to CSS.

use std::collections::BTreeMap;

use adblock::Engine;
use adblock::cosmetic_filter_cache::{ProceduralOrActionFilter, UrlSpecificResources};

use crate::list::List;
use crate::matching::Choices;
use crate::rules::FilterLists;

/// One cosmetic rule that hides through CSS: its selector, and the
/// declarations it applies, `display: none !important` unless a rule's
/// `:style()` names others.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CssRule {
    /// Its selector.
    pub selector: String,
    /// Its declarations.
    pub style: String,
    /// The list it comes from.
    pub list: List,
}

/// The declarations a hiding rule applies.
pub(crate) const HIDDEN: &str = "display: none !important";

/// The cosmetic rules each list that is on holds for one page's URL.
pub struct PageRules<'a> {
    lists: Vec<(List, &'a Engine, UrlSpecificResources)>,
}

impl<'a> PageRules<'a> {
    /// The rules of the lists `choices` turn on, among `lists`, for the
    /// page at `url`.
    pub fn of(lists: &'a FilterLists, choices: &Choices, url: &str) -> Self {
        let lists = [
            (List::Ads, choices.ads, &lists.ads),
            (List::Trackers, choices.trackers, &lists.trackers),
            (List::Annoyances, choices.annoyances, &lists.annoyances),
        ]
        .into_iter()
        .filter(|(_, on, _)| *on)
        .map(|(list, _, engine)| (list, engine, engine.url_cosmetic_resources(url)))
        .collect();
        Self { lists }
    }

    /// Whether no list is on.
    pub fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }

    /// The rules that hide through CSS on the page while it holds
    /// `classes` and `ids`: its URL's plain selectors, the generic ones its
    /// classes and IDs key, and the procedural rules that are plain CSS,
    /// each selector once, from the first list that has it, in selector
    /// order.
    pub fn css_rules(&self, classes: &[&str], ids: &[&str]) -> Vec<CssRule> {
        let mut rules: BTreeMap<String, (String, List)> = BTreeMap::new();
        for (list, engine, resources) in &self.lists {
            let mut add = |selector: String, style: String| {
                rules.entry(selector).or_insert((style, *list));
            };
            #[expect(
                clippy::iter_over_hash_type,
                reason = "each selector goes into an ordered map, so the order read is not the order kept"
            )]
            for selector in &resources.hide_selectors {
                add(selector.clone(), HIDDEN.to_owned());
            }
            if !resources.generichide {
                for selector in engine.hidden_class_id_selectors(
                    classes.iter(),
                    ids.iter(),
                    &resources.exceptions,
                ) {
                    add(selector, HIDDEN.to_owned());
                }
            }
            for filter in procedural(resources) {
                if let Some((selector, style)) = filter.as_css() {
                    add(selector, style);
                }
            }
        }
        rules
            .into_iter()
            .map(|(selector, (style, list))| CssRule {
                selector,
                style,
                list,
            })
            .collect()
    }
}

/// The procedural rules `resources` hold, each read from the JSON `adblock`
/// keeps them in; one it cannot read is left out.
pub(crate) fn procedural(
    resources: &UrlSpecificResources,
) -> impl Iterator<Item = ProceduralOrActionFilter> {
    let mut filters: Vec<&String> = resources.procedural_actions.iter().collect();
    filters.sort();
    filters
        .into_iter()
        .filter_map(|json| serde_json::from_str(json).ok())
}
