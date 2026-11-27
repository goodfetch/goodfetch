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
        let lists = lists
            .on(choices.lists())
            .map(|(list, engine)| (list, engine, engine.url_cosmetic_resources(url)))
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

/// A batch of resolved `+js(...)` scriptlets for one document, together with
/// the invocation lines `adblock` emitted (for Provenance recording).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptletBatch {
    /// JavaScript to evaluate in the document's main world before page scripts.
    pub script: String,
    /// Each scriptlet invocation emitted inside `try { … } catch ( e ) { }`.
    pub invocations: Vec<String>,
}

impl ScriptletBatch {
    /// Whether no scriptlet matched.
    pub fn is_empty(&self) -> bool {
        self.script.trim().is_empty()
    }
}

/// The scriptlets matching a page's URL, split between the built-in Filter
/// lists and `filter_lists.extra` so Provenance attributes extra-list
/// scriptlets to the configuration layer that added them (ADR 0047).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageScriptlets {
    /// Scriptlets from the built-in Filter lists that are on.
    pub built_in: ScriptletBatch,
    /// Scriptlets from `filter_lists.extra` (empty unless `allow_extra` was
    /// true for this Run's layer).
    pub extra: ScriptletBatch,
}

impl PageScriptlets {
    /// Resolves scriptlets for the page at `url` from `lists` under `choices`.
    /// Extra-list scriptlets are included only when `allow_extra` is `true`
    /// (Operator policy, user or project configuration, never Recipe/Run/agent
    /// surfaces; ADR 0047).
    pub fn of(lists: &FilterLists, choices: &Choices, url: &str, allow_extra: bool) -> Self {
        let mut built_in_script = String::new();
        let mut built_in_invocations = Vec::new();
        let on = choices.lists();
        for (list, engine) in lists.engines() {
            if !on.contains(list) {
                continue;
            }
            let injected = engine.url_cosmetic_resources(url).injected_script;
            if !injected.trim().is_empty() {
                built_in_invocations.extend(FilterLists::scriptlet_invocations(&injected));
                built_in_script.push_str(&injected);
                built_in_script.push('\n');
            }
        }
        let mut extra_script = String::new();
        let mut extra_invocations = Vec::new();
        if allow_extra
            && choices.extra
            && let Some(engine) = lists.extra_engine()
        {
            let injected = engine.url_cosmetic_resources(url).injected_script;
            if !injected.trim().is_empty() {
                extra_invocations.extend(FilterLists::scriptlet_invocations(&injected));
                extra_script.push_str(&injected);
                extra_script.push('\n');
            }
        }
        Self {
            built_in: ScriptletBatch {
                script: built_in_script,
                invocations: built_in_invocations,
            },
            extra: ScriptletBatch {
                script: extra_script,
                invocations: extra_invocations,
            },
        }
    }

    /// Whether neither built-in nor extra lists produced any scriptlet.
    pub fn is_empty(&self) -> bool {
        self.built_in.is_empty() && self.extra.is_empty()
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
