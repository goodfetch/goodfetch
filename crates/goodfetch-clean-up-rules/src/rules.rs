//! The embedded Filter lists (ADRs 0047 and 0057): one `adblock` engine
//! per list, each serialized at build time and embedded zstd-compressed,
//! decompressed when loaded and dropped once deserialized. A loaded
//! [`FilterLists`] is `Send + Sync` (`adblock` without its `single-thread`
//! feature), so every Session shares one behind an `Arc` with no lock.

use adblock::Engine;

use crate::filter_set;
use crate::generic::Generic;
use crate::list::{LISTS, List};
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
/// built with them, once per version, off every page's path (issue 2191).
pub struct FilterLists {
    pub(crate) ads: Engine,
    pub(crate) trackers: Engine,
    pub(crate) annoyances: Engine,
    /// The `$generichide` exceptions of each list, in [`List`] order.
    generic_hide: [Engine; 3],
    generic: Generic,
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
    /// The lists put together held no engine of one list.
    #[error("the embedded Filter lists were put together without `{list}`")]
    Missing {
        /// The list.
        list: &'static str,
    },
}

/// One embedded Filter list's engine, loaded on its own, so that the engine
/// loads the lists side by side on its blocking threads (#2688) before
/// [`FilterLists::of_embedded`] puts them together.
pub struct EmbeddedList {
    list: List,
    engine: Engine,
}

impl EmbeddedList {
    /// Loads the engine of `list` from its zstd form into a buffer of its
    /// own, which goes once the engine holds its own copy. It blocks for
    /// some tens of milliseconds, so the engine runs it on a blocking task
    /// (ADR 0060).
    ///
    /// # Errors
    ///
    /// The list does not decompress or deserialize: a broken build.
    pub fn load(list: List) -> Result<Self, LoadError> {
        Self::load_into(list, &mut Vec::with_capacity(compressed(list).size))
    }

    /// Loads the engine of `list`, decompressed into `buffer`.
    fn load_into(list: List, buffer: &mut Vec<u8>) -> Result<Self, LoadError> {
        let engine = load(list.name(), compressed(list), buffer)?;
        Ok(Self { list, engine })
    }

    /// Its list.
    pub fn list(&self) -> List {
        self.list
    }
}

impl FilterLists {
    /// Loads the embedded Filter lists one after another, on the calling
    /// thread, each decompressed into one buffer, which the next list reuses
    /// and which goes once the last engine holds its own copy, so loading
    /// holds at most the largest list's bytes beside the engines. It
    /// blocks for some tens of milliseconds, so the engine runs it on a
    /// blocking task (ADR 0060); the engine itself loads the lists side by
    /// side with [`EmbeddedList::load`] and [`FilterLists::of_embedded`].
    ///
    /// # Errors
    ///
    /// A list that does not decompress or deserialize: a broken build.
    pub fn embedded() -> Result<Self, LoadError> {
        let largest = LISTS
            .map(|list| compressed(list).size)
            .into_iter()
            .max()
            .unwrap_or_default();
        let mut buffer = Vec::with_capacity(largest);
        let mut loaded = Vec::with_capacity(LISTS.len());
        for list in LISTS {
            loaded.push(EmbeddedList::load_into(list, &mut buffer)?);
        }
        // Each engine holds its own copy: the decompressed bytes go now.
        drop(buffer);
        Self::of_embedded(loaded)
    }

    /// The embedded Filter lists from `loaded`, one engine of each list in
    /// any order, with each list's `$generichide` exceptions and their
    /// generic set compiled beside them. It blocks for some milliseconds, so
    /// the engine runs it on a blocking task (ADR 0060).
    ///
    /// # Errors
    ///
    /// `loaded` holds no engine of a list.
    pub fn of_embedded(loaded: impl IntoIterator<Item = EmbeddedList>) -> Result<Self, LoadError> {
        let mut engines: [Option<Engine>; 3] = [None, None, None];
        for EmbeddedList { list, engine } in loaded {
            engines[at(list)] = Some(engine);
        }
        let [ads, trackers, annoyances] = engines;
        let present = |engine: Option<Engine>, list: List| {
            engine.ok_or(LoadError::Missing { list: list.name() })
        };
        Ok(Self::of(
            [
                present(ads, List::Ads)?,
                present(trackers, List::Trackers)?,
                present(annoyances, List::Annoyances)?,
            ],
            [
                ADS_GENERIC_HIDE,
                TRACKERS_GENERIC_HIDE,
                ANNOYANCES_GENERIC_HIDE,
            ]
            .map(|lines| generic_hide(lines.to_owned(), UNBREAK_GENERIC_HIDE.to_owned())),
        ))
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
        Self::of(
            [texts.ads, texts.trackers, texts.annoyances].map(compiled),
            [texts.ads, texts.trackers, texts.annoyances]
                .map(|list| generic_hide(filter_set::generic_hide_lines(list), unbreak.clone())),
        )
    }

    /// The lists of the engines `[ads, trackers, annoyances]` and their
    /// `$generichide` engines, in [`List`] order, with their generic set.
    fn of([ads, trackers, annoyances]: [Engine; 3], generic_hide: [Engine; 3]) -> Self {
        let generic = Generic::of([
            (List::Ads, &ads),
            (List::Trackers, &trackers),
            (List::Annoyances, &annoyances),
        ]);
        Self {
            ads,
            trackers,
            annoyances,
            generic_hide,
            generic,
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
        self.generic_hide[at(list)]
            .url_cosmetic_resources(url)
            .generichide
    }

    /// The generic set removal reads every page with.
    pub(crate) fn generic(&self) -> &Generic {
        &self.generic
    }
}

/// Where `list` stands in [`List`] order.
fn at(list: List) -> usize {
    match list {
        List::Ads => 0,
        List::Trackers => 1,
        List::Annoyances => 2,
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

/// The compressed form of `list`.
fn compressed(list: List) -> &'static Compressed {
    match list {
        List::Ads => &ADS,
        List::Trackers => &TRACKERS,
        List::Annoyances => &ANNOYANCES,
    }
}

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
