//! The embedded Filter lists (ADRs 0047 and 0057): one `adblock` engine
//! per list, each serialized at build time and embedded zstd-compressed,
//! decompressed when loaded and dropped once deserialized. A loaded
//! [`FilterLists`] is `Send + Sync` (`adblock` without its `single-thread`
//! feature), so every Session shares one behind an `Arc` with no lock.

use adblock::Engine;

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
/// Annoyance, each with Brave's unbreak list, without the consent-vendor
/// entries held back until the Consent prompt runner is done.
pub struct FilterLists {
    pub(crate) ads: Engine,
    pub(crate) trackers: Engine,
    pub(crate) annoyances: Engine,
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
        };
        // Each engine holds its own copy: the decompressed bytes go now.
        drop(buffer);
        Ok(lists)
    }

    /// The size of the embedded lists' serialized forms, together.
    pub fn serialized_size() -> usize {
        ADS.size + TRACKERS.size + ANNOYANCES.size
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
