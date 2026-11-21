//! The Consent prompt runner's bundle (ADR 0048): autoconsent's library and
//! heuristics (MPL-2.0, unmodified), its rules less every rule that names a
//! dropped snippet, Goodfetch's snippet table and Goodfetch's runner, built
//! into one script at build time and embedded zstd-compressed (ADR 0057).
//! The engine evaluates it in the guard's isolated world of every frame,
//! starts it once it added the world's binding, and runs in a frame's main
//! world only the snippets [`world`] classes [`World::Main`].

include!(concat!(env!("OUT_DIR"), "/consent.rs"));

/// Where one of autoconsent's `eval` snippets runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum World {
    /// The guard's isolated world: it reaches only the DOM, cookies,
    /// storage or history.
    Isolated,
    /// The frame's main world: it reads or calls a page global, such as a
    /// vendor's own object.
    Main,
    /// Nowhere: it clicks through a synthetic event or from the main world,
    /// which no trusted input makes, so every rule naming it is dropped.
    Dropped,
}

/// The world the snippet table classes `snippet` in; `None` for a snippet
/// the pinned library does not have.
pub fn world(snippet: &str) -> Option<World> {
    SNIPPETS
        .iter()
        .find(|(name, _)| *name == snippet)
        .map(|(_, world)| *world)
}

/// The runner's bundle, decompressed: the script the engine evaluates in
/// the guard's isolated world of every frame at every new document.
#[derive(Debug)]
pub struct Runner {
    source: String,
}

/// Why the embedded bundle did not load: never once the build wrote it,
/// so a failure names a broken build.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoadError {
    /// It did not decompress.
    #[error("the Consent prompt runner's bundle did not decompress: {0}")]
    Decompress(std::io::Error),
    /// It is not UTF-8.
    #[error("the Consent prompt runner's bundle is not UTF-8: {0}")]
    Text(std::string::FromUtf8Error),
}

impl Runner {
    /// Decompresses the embedded bundle, about 0.5 MB, in a few
    /// milliseconds, so the engine runs it on a blocking task (ADR 0060).
    ///
    /// # Errors
    ///
    /// A bundle that does not decompress, or is not UTF-8: a broken build.
    pub fn embedded() -> Result<Self, LoadError> {
        let bytes = zstd::bulk::decompress(BUNDLE, BUNDLE_SIZE).map_err(LoadError::Decompress)?;
        String::from_utf8(bytes)
            .map(|source| Self { source })
            .map_err(LoadError::Text)
    }

    /// The script.
    pub fn source(&self) -> &str {
        &self.source
    }
}
