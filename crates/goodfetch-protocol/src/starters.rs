//! What `init` writes (spec #608, issue 1422; ADRs 0087, 0096 and 0097): a
//! starter, a project an executable embeds, in one of the SDKs' languages,
//! and the result `init --json` prints.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The starters only Foxhound's executable embeds (ADRs 0087 and 0097),
/// which Collie's refuses, naming Foxhound's.
pub const FOXHOUND_STARTERS: [&str; 1] = ["crawl"];

/// A language `init` writes a starter in: one of the SDKs'.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum StarterLanguage {
    /// TypeScript, on the `@goodfetch/collie` package.
    Typescript,
    /// Python, on the `goodfetch-collie` package.
    Python,
    /// Rust, on the `goodfetch-collie` crate.
    Rust,
}

impl StarterLanguage {
    /// Every language, in the order `init` offers them.
    pub const ALL: [Self; 3] = [Self::Typescript, Self::Python, Self::Rust];

    /// Its name on the command line, in [`InitResult`] and in the path of
    /// its starters' files.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Typescript => "typescript",
            Self::Python => "python",
            Self::Rust => "rust",
        }
    }

    /// The language this name names, if any.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.as_str() == name)
    }
}

/// What `init --json` prints: the starter it wrote, in which language, and
/// each of its files by its path from the project's folder, with `/`
/// between its parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InitResult {
    /// The starter's name, such as `browser`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub starter: String,
    /// Its language.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub language: StarterLanguage,
    /// The files it wrote.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub written: Vec<String>,
    /// The files it left as they were, because the path was already taken:
    /// `init` never overwrites one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub skipped: Vec<String>,
}
