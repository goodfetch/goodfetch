//! The embedded Filter lists (ADRs 0047 and 0057): one `adblock` engine
//! per list, each serialized at build time and embedded zstd-compressed,
//! decompressed when loaded and dropped once deserialized. A loaded
//! [`FilterLists`] is `Send + Sync` (`adblock` without its `single-thread`
//! feature), so every Session shares one behind an `Arc` with no lock.

use std::sync::OnceLock;

use adblock::Engine;

use crate::filter_set;
use crate::generic::Generic;
use crate::list::{LISTS, List, ListSet};
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
/// The removal-only cookie-section engines of a version, together with
/// their generic set, loaded on first extraction-side removal for the
/// embedded part so Runs that never extract pay neither its memory nor its
/// load time (ADR 0047, issue 2379).
pub(crate) struct RemovalOnly {
    engines: [Option<Engine>; 3],
    generic: Generic,
}

impl RemovalOnly {
    fn of(engines: [Option<Engine>; 3]) -> Self {
        let generic = Generic::of_optional([
            (List::Ads, engines[0].as_ref()),
            (List::Trackers, engines[1].as_ref()),
            (List::Annoyances, engines[2].as_ref()),
        ]);
        Self { engines, generic }
    }

    fn load_embedded() -> Self {
        let largest = LISTS
            .into_iter()
            .filter_map(removal_compressed)
            .map(|extra| extra.size)
            .max()
            .unwrap_or_default();
        let mut buffer = Vec::with_capacity(largest);
        let engines = LISTS.map(|list| {
            removal_compressed(list).map(|extra| {
                load_cosmetic(list.name(), extra, &mut buffer).expect(
                    "invariant: the build compiled the embedded removal-only cookie engine with this binary",
                )
            })
        });
        drop(buffer);
        Self::of(engines)
    }

    /// The removal-only engine of `list`, when it has one.
    pub(crate) fn engine(&self, list: List) -> Option<&Engine> {
        self.engines[at(list)].as_ref()
    }

    /// The generic set of the removal-only cookie-section engines.
    pub(crate) fn generic(&self) -> &Generic {
        &self.generic
    }
}

pub struct FilterLists {
    ads: Engine,
    trackers: Engine,
    annoyances: Engine,
    /// Removal-only engines compiled from the cosmetic rules of each list's
    /// EasyList Cookie sections (ADR 0047, issue 2379), in [`List`] order:
    /// read only by extraction-side removal, never by network blocking or
    /// live hiding. Pre-filled by [`FilterLists::compile`]; loaded on first
    /// removal for the embedded part.
    removal_only: OnceLock<RemovalOnly>,
    /// The `$generichide` exceptions of each list, in [`List`] order.
    generic_hide: [Engine; 3],
    /// The `$generichide` exceptions of each list's cookie sections, in
    /// [`List`] order, when any exist.
    removal_generic_hide: [Option<Engine>; 3],
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
            OnceLock::new(),
            [
                ADS_GENERIC_HIDE,
                TRACKERS_GENERIC_HIDE,
                ANNOYANCES_GENERIC_HIDE,
            ]
            .map(|lines| generic_hide(lines.to_owned(), UNBREAK_GENERIC_HIDE.to_owned())),
            [
                ADS_REMOVAL_GENERIC_HIDE,
                TRACKERS_REMOVAL_GENERIC_HIDE,
                ANNOYANCES_REMOVAL_GENERIC_HIDE,
            ]
            .map(optional_generic_hide),
        ))
    }

    /// Compiles a downloaded version's Filter lists from their texts, each
    /// with the version's unbreak list, as the build compiles the embedded
    /// ones: on the engine's own machine, with its own `adblock` (ADR 0047).
    /// Cookie sections are split into removal-only cosmetic engines so they
    /// never block requests or hide elements on the live page (issue 2379).
    /// It blocks for as long as the lists are long, so the engine runs it
    /// on a blocking task (ADR 0060).
    pub fn compile(texts: &ListTexts<'_>) -> Self {
        let unbreak = filter_set::generic_hide_lines(texts.unbreak);
        let splits =
            [texts.ads, texts.trackers, texts.annoyances].map(filter_set::split_cookie_sections);
        let live_generic = splits.each_ref().map(|split| {
            generic_hide(filter_set::generic_hide_lines(&split.live), unbreak.clone())
        });
        let removal_generic = splits.each_ref().map(|split| {
            let lines = filter_set::generic_hide_lines(&split.cookies);
            optional_generic_hide(&lines)
        });
        let removal_only = OnceLock::from(RemovalOnly::of(splits.each_ref().map(|split| {
            filter_set::removal_only(&split.cookies, texts.unbreak).map(Engine::new_with_filter_set)
        })));
        let live = splits.map(|split| {
            let set = filter_set::of(split.live, texts.unbreak.to_owned());
            let mut engine = Engine::new_with_filter_set(set);
            engine.use_resources(stand_ins::resources());
            engine
        });
        Self::of(live, removal_only, live_generic, removal_generic)
    }

    /// The lists of the engines `[ads, trackers, annoyances]`, their
    /// removal-only cookie-section engines, and their `$generichide`
    /// engines, in [`List`] order, with their generic set.
    fn of(
        [ads, trackers, annoyances]: [Engine; 3],
        removal_only: OnceLock<RemovalOnly>,
        generic_hide: [Engine; 3],
        removal_generic_hide: [Option<Engine>; 3],
    ) -> Self {
        let generic = Generic::of([
            (List::Ads, &ads),
            (List::Trackers, &trackers),
            (List::Annoyances, &annoyances),
        ]);
        Self {
            ads,
            trackers,
            annoyances,
            removal_only,
            generic_hide,
            removal_generic_hide,
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

    /// The engine of each list of `on`, in [`List`] order.
    pub(crate) fn on(&self, on: ListSet) -> impl Iterator<Item = (List, &Engine)> {
        self.engines()
            .into_iter()
            .filter(move |(list, _)| on.contains(*list))
    }

    /// The engine of `list`.
    pub(crate) fn engine(&self, list: List) -> &Engine {
        match list {
            List::Ads => &self.ads,
            List::Trackers => &self.trackers,
            List::Annoyances => &self.annoyances,
        }
    }

    /// The removal-only cookie-section engines and their generic set, loaded
    /// on first call for the embedded part.
    pub(crate) fn removal(&self) -> &RemovalOnly {
        self.removal_only.get_or_init(RemovalOnly::load_embedded)
    }

    /// The removal-only cookie-section engine of `list`, when it has one.
    pub(crate) fn removal_only(&self, list: List) -> Option<&Engine> {
        self.removal().engine(list)
    }

    /// Whether a `$generichide` exception of `list`, or of the unbreak
    /// list, turns the list's generic rules off for the page at `url`, as
    /// the list's engine answers it.
    pub(crate) fn generic_hide(&self, list: List, url: &str) -> bool {
        self.generic_hide[at(list)]
            .url_cosmetic_resources(url)
            .generichide
    }

    /// Whether a `$generichide` exception of `list`, its cookie sections,
    /// or the unbreak list turns the list's removal-only generic rules off
    /// for the page at `url`.
    pub(crate) fn removal_generic_hide(&self, list: List, url: &str) -> bool {
        self.generic_hide(list, url)
            || self.removal_generic_hide[at(list)]
                .as_ref()
                .is_some_and(|engine| engine.url_cosmetic_resources(url).generichide)
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

/// A small `$generichide` engine when `lines` is non-empty.
fn optional_generic_hide(lines: &str) -> Option<Engine> {
    (!lines.is_empty()).then(|| generic_hide(lines.to_owned(), String::new()))
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
/// sections: a section begins at its marker,
/// `! *** easylist:easylist_cookie/… ***` in easylist.to's builds or
/// `! easylist_cookie_….txt` in fanboy.co.nz's, and runs to the next marker
/// of either form.
pub fn without_cookie_sections(text: &str) -> String {
    filter_set::split_cookie_sections(text).live
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

/// The compressed removal-only cookie-section engine of `list`, when it
/// carries any.
fn removal_compressed(list: List) -> Option<&'static Compressed> {
    match list {
        List::Ads => ADS_REMOVAL_ONLY.as_ref(),
        List::Trackers => TRACKERS_REMOVAL_ONLY.as_ref(),
        List::Annoyances => ANNOYANCES_REMOVAL_ONLY.as_ref(),
    }
}

/// Loads the engine of `list` from `compressed`, decompressed into
/// `buffer`, with the stand-ins.
fn load(
    list: &'static str,
    compressed: &Compressed,
    buffer: &mut Vec<u8>,
) -> Result<Engine, LoadError> {
    let mut engine = load_cosmetic(list, compressed, buffer)?;
    engine.use_resources(stand_ins::resources());
    Ok(engine)
}

/// Loads a cosmetic-only engine from `compressed`, decompressed into
/// `buffer`, without loading redirect stand-ins.
fn load_cosmetic(
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
    Ok(engine)
}
