//! The embedded Filter lists (ADRs 0047 and 0057): one `adblock` engine
//! per list, each serialized at build time and embedded zstd-compressed,
//! decompressed when loaded and dropped once deserialized. A loaded
//! [`FilterLists`] is `Send + Sync` (`adblock` without its `single-thread`
//! feature), so every Session shares one behind an `Arc` with no lock.

use std::sync::OnceLock;

use adblock::Engine;

use crate::filter_set;
use crate::generic::Generic;
use crate::list::List;
use crate::stand_ins;

/// One embedded engine as the build wrote it.
pub(crate) struct Compressed {
    /// Its serialized form, zstd-compressed.
    pub(crate) bytes: &'static [u8],
    /// The size of its serialized form.
    pub(crate) size: usize,
}

include!(concat!(env!("OUT_DIR"), "/embedded.rs"));

/// The embedded Filter lists, loaded: EasyList, EasyPrivacy and Fanboy
/// Annoyance, each with Brave's unbreak list; the consent vendors' entries
/// stay in, each joining the never-block list (#717). Beside each list's
/// engine, a small one holds its and the unbreak list's `$generichide`
/// exceptions alone, which answer whether a page's URL turns the list's
/// generic rules off; and the generic set removal reads every page with is
/// built once, on first use.
pub struct FilterLists {
    pub(crate) ads: Engine,
    pub(crate) trackers: Engine,
    pub(crate) annoyances: Engine,
    /// The `$generichide` exceptions of each list, in [`List`] order.
    generic_hide: [Engine; 3],
    generic: OnceLock<Generic>,
}

/// Why the embedded Filter lists did not load: never once the build
/// compiled them with this binary's `adblock`, so a failure names a broken
/// build.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// A list's compressed form did not decompress.
    #[error("the embedded `{list}` Filter list did not decompress: {source}")]
    Decompress {
        /// The list.
        list: &'static str,
        /// Why.
        source: std::io::Error,
    },
    /// A list's serialized form did not deserialize.
    #[error("the embedded `{list}` Filter list did not deserialize: {reason}")]
    Deserialize {
        /// The list.
        list: &'static str,
        /// Why, as `adblock` says.
        reason: String,
    },
}

impl FilterLists {
    /// Loads the embedded Filter lists. It blocks for some tens of
    /// milliseconds, so the engine runs it on a blocking task (ADR 0060).
    /// Each list is decompressed into one buffer, which the next list
    /// reuses and which goes once the last engine holds its own copy, so
    /// loading holds at most the largest list's bytes beside the engines.
    ///
    /// # Errors
    ///
    /// A list that does not decompress or deserialize: a broken build.
    pub fn embedded() -> Result<Self, LoadError> {
        let largest = [&ADS, &TRACKERS, &ANNOYANCES]
            .iter()
            .map(|list| list.size)
            .max()
            .unwrap_or_default();
        let mut buffer = Vec::with_capacity(largest);
        let lists = Self {
            ads: load("ads", &ADS, &mut buffer)?,
            trackers: load("trackers", &TRACKERS, &mut buffer)?,
            annoyances: load("annoyances", &ANNOYANCES, &mut buffer)?,
            generic_hide: [
                ADS_GENERIC_HIDE,
                TRACKERS_GENERIC_HIDE,
                ANNOYANCES_GENERIC_HIDE,
            ]
            .map(|lines| generic_hide(lines.to_owned(), UNBREAK_GENERIC_HIDE.to_owned())),
            generic: OnceLock::new(),
        };
        // Each engine holds its own copy: the decompressed bytes go now.
        drop(buffer);
        Ok(lists)
    }

    /// The size of the embedded lists' serialized forms, together.
    pub fn serialized_size() -> usize {
        ADS.size + TRACKERS.size + ANNOYANCES.size
    }

    /// Compiles a downloaded version's Filter lists from their texts, each
    /// with the version's unbreak list, as the build compiles the embedded
    /// ones: on the engine's own machine, with its own `adblock` (ADR 0047).
    /// It blocks for as long as the lists are long, so the engine runs it
    /// on a blocking task (ADR 0060).
    pub fn compile(texts: &ListTexts<'_>) -> Self {
        let compiled = |list: &str| {
            let set = filter_set::of(list.to_owned(), texts.unbreak.to_owned());
            let mut engine = Engine::new_with_filter_set(set);
            engine.use_resources(stand_ins::resources());
            engine
        };
        let unbreak = filter_set::generic_hide_lines(texts.unbreak);
        Self {
            ads: compiled(texts.ads),
            trackers: compiled(texts.trackers),
            annoyances: compiled(texts.annoyances),
            generic_hide: [texts.ads, texts.trackers, texts.annoyances]
                .map(|list| generic_hide(filter_set::generic_hide_lines(list), unbreak.clone())),
            generic: OnceLock::new(),
        }
    }

    /// Each list's engine, in [`List`] order.
    pub(crate) fn engines(&self) -> [(List, &Engine); 3] {
        [
            (List::Ads, &self.ads),
            (List::Trackers, &self.trackers),
            (List::Annoyances, &self.annoyances),
        ]
    }

    /// The engine of `list`.
    pub(crate) fn engine(&self, list: List) -> &Engine {
        match list {
            List::Ads => &self.ads,
            List::Trackers => &self.trackers,
            List::Annoyances => &self.annoyances,
        }
    }

    /// Whether a `$generichide` exception of `list`, or of the unbreak
    /// list, turns the list's generic rules off for the page at `url`, as
    /// the list's engine answers it.
    pub(crate) fn generic_hide(&self, list: List, url: &str) -> bool {
        let at = match list {
            List::Ads => 0,
            List::Trackers => 1,
            List::Annoyances => 2,
        };
        self.generic_hide[at]
            .url_cosmetic_resources(url)
            .generichide
    }

    /// The generic set removal reads every page with, built on first use.
    pub(crate) fn generic(&self) -> &Generic {
        self.generic.get_or_init(|| Generic::of(self.engines()))
    }
}

/// The engine of `lines`, a list's `$generichide` exceptions, and
/// `unbreak`, the unbreak list's.
fn generic_hide(lines: String, unbreak: String) -> Engine {
    Engine::new_with_filter_set(filter_set::of(lines, unbreak))
}

/// The texts of one version's Filter lists.
#[derive(Clone, Copy, Debug)]
pub struct ListTexts<'a> {
    /// EasyList's, which `filter_lists.ads` turns on.
    pub ads: &'a str,
    /// EasyPrivacy's, which `filter_lists.trackers` turns on.
    pub trackers: &'a str,
    /// Fanboy Annoyance's, which `filter_lists.annoyances` turns on.
    pub annoyances: &'a str,
    /// Brave's unbreak list, compiled into each.
    pub unbreak: &'a str,
}

/// `text`, a Filter list Goodfetch did not build, without its cookie
/// sections, which the Consent prompt runner handles instead (ADR 0047): a
/// section begins at its marker, `! *** easylist:easylist_cookie/… ***` in
/// easylist.to's builds or `! easylist_cookie_….txt` in fanboy.co.nz's, and
/// runs to the next marker of either form.
pub fn without_cookie_sections(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut dropping = false;
    for line in text.split_inclusive('\n') {
        if let Some(section) = section_marker(line.trim_end()) {
            dropping = section.contains("easylist_cookie");
        }
        if !dropping {
            kept.push_str(line);
        }
    }
    kept
}

/// The section a marker line names: `easylist:<path>` in `! *** … ***`, or
/// the `<name>.txt` of `! <name>.txt`; none for any other line.
fn section_marker(line: &str) -> Option<&str> {
    let comment = line.strip_prefix('!')?.trim();
    if let Some(inner) = comment
        .strip_prefix("***")
        .and_then(|rest| rest.strip_suffix("***"))
    {
        return Some(inner.trim());
    }
    let name = comment.strip_suffix(".txt")?;
    (!name.is_empty() && !name.contains(char::is_whitespace)).then_some(comment)
}

/// The digest of the embedded part's files, its `manifest.json` among them,
/// `sha256:<hex>` by the Browser cache's tree rule (ADR 0024): the version
/// a Run holds to when it holds to the embedded part.
pub const EMBEDDED_DIGEST: &str = EMBEDDED_DIGEST_OF_BUILD;

/// The embedded part's `manifest.json`, as the crate carries it.
pub const EMBEDDED_MANIFEST: &str = include_str!("../rules/manifest.json");

/// Loads the engine of `list` from `compressed`, decompressed into
/// `buffer`, with the stand-ins.
fn load(
    list: &'static str,
    compressed: &Compressed,
    buffer: &mut Vec<u8>,
) -> Result<Engine, LoadError> {
    buffer.clear();
    zstd::bulk::Decompressor::new()
        .and_then(|mut decompressor| decompressor.decompress_to_buffer(compressed.bytes, buffer))
        .map_err(|source| LoadError::Decompress { list, source })?;
    let mut engine = Engine::default();
    engine
        .deserialize(buffer)
        .map_err(|error| LoadError::Deserialize {
            list,
            reason: error.to_string(),
        })?;
    engine.use_resources(stand_ins::resources());
    Ok(engine)
}
