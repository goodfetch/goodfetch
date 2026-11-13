//! The lowest-minor marker of the shared stores (ADR 0098): the Run index,
//! the Schedule store, the Saved identity store, user configuration and the
//! Operator policy, which engines of several minors of one Product major,
//! of either Product, open at once. Beside its format's version each
//! records the lowest minor of its Product major that can use it.
//!
//! - An engine that creates a store marks it usable by every minor of its
//!   major.
//! - A newer engine raises the lowest minor only when it writes what an
//!   older engine would misread or misapply, to the minor that added it:
//!   in user configuration and the Operator policy, any key its minor
//!   added. It also records its newer format version.
//! - An engine opens a store whose lowest minor it meets, its format's
//!   newer minors included, keeping what it does not know untouched, and
//!   refuses one marked newer than itself, naming the version to install.
//! - The marker is read in each transaction, so an engine that finds it
//!   raised while it runs stops using the store with the same refusal.
//!
//! During `0.y` previews each `0.y` minor counts as a major (ADR 0030): a
//! preview's line has the minor 0 alone, so its engines never raise it.
//!
//! A store keeps the marker in its own storage: [`read_sqlite`] and
//! [`write_sqlite`] in a SQLite store's `format` table, and the settings
//! files beside their `version`. The areas that build the Schedule store
//! and the Saved identity store register them under
//! [`Rule::SharedStore`](goodfetch_protocol::formats::Rule::SharedStore)
//! and open them through an [`Opener`].

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, Format, line_minor};
use rusqlite::{Connection, OptionalExtension as _};

/// What a shared store records of its version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    /// Its format's version, the newest of the engines that wrote it.
    pub version: ProtocolVersion,
    /// The lowest minor of its Product major that can use it.
    pub lowest_minor: u16,
}

/// Why an engine does not use a shared store: its format, of a major this
/// engine does not read, or its lowest minor, newer than the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The format's name.
    pub format: &'static str,
    /// The store's format version.
    pub found: ProtocolVersion,
    /// What the engine makes of it.
    pub compatibility: Compatibility,
    /// The Product version to install, when the lowest minor refused it.
    pub install: Option<String>,
}

/// An engine that opens a shared store: the store's format as the engine
/// knows it, and the engine's own Product major and minor.
#[derive(Clone, Copy)]
pub struct Opener {
    format: &'static Format,
    engine: ProtocolVersion,
}

impl std::fmt::Debug for Opener {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Opener")
            .field("format", &self.format.name)
            .field("version", &self.format.version)
            .field("engine", &self.engine)
            .finish()
    }
}

impl Opener {
    /// The engine of Product version `engine` (its major and minor) that
    /// knows `format`, a shared store's.
    pub fn new(format: &'static Format, engine: ProtocolVersion) -> Self {
        Self { format, engine }
    }

    /// The format as this engine knows it.
    pub fn format(&self) -> &'static Format {
        self.format
    }

    /// The marker of a store this engine creates: its own format version,
    /// usable by every minor of its major.
    pub fn created(&self) -> Marker {
        Marker {
            version: self.format.version,
            lowest_minor: 0,
        }
    }

    /// Whether this engine may use a store marked `found`.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] of a store of another format major, or one whose
    /// lowest minor is newer than this engine.
    pub fn check(&self, found: &Marker) -> Result<(), Refusal> {
        let compatibility = self.format.rule.compatibility_of_store(
            self.format.version,
            found.version,
            self.engine,
            found.lowest_minor,
        );
        let install = match compatibility {
            Compatibility::Compatible => return Ok(()),
            Compatibility::NewerMinor => Some(self.install(found.lowest_minor)),
            Compatibility::OlderMajor | Compatibility::NewerMajor => None,
        };
        Err(Refusal {
            format: self.format.name,
            found: found.version,
            compatibility,
            install,
        })
    }

    /// The marker after this engine writes to a store marked `found`, the
    /// write needing what the Product version `needs` (major and minor)
    /// added, if anything an older engine would misread.
    pub fn written(&self, found: Marker, needs: Option<ProtocolVersion>) -> Marker {
        let version = if self.format.version.major == found.version.major {
            self.format.version.max(found.version)
        } else {
            found.version
        };
        let raised = needs
            .filter(|needs| line(*needs) == line(self.engine))
            .map_or(0, line_minor);
        Marker {
            version,
            lowest_minor: found.lowest_minor.max(raised),
        }
    }

    /// The Product version that meets `lowest_minor` in this engine's line.
    fn install(&self, lowest_minor: u16) -> String {
        if self.engine.major == 0 {
            format!("0.{}.{lowest_minor}", self.engine.minor)
        } else {
            format!("{}.{lowest_minor}", self.engine.major)
        }
    }
}

/// A Product version's line: its major, or `0.y` during previews.
fn line(version: ProtocolVersion) -> (u16, u16) {
    if version.major == 0 {
        (0, version.minor)
    } else {
        (version.major, 0)
    }
}

/// The columns a SQLite store's `format` table adds for the marker.
pub(crate) const FORMAT_TABLE: &str = "CREATE TABLE IF NOT EXISTS format (
    name TEXT NOT NULL PRIMARY KEY,
    version TEXT NOT NULL,
    lowest_minor INTEGER NOT NULL DEFAULT 0
) STRICT";

/// The marker of the store whose format is `name`, from its `format`
/// table; a version that does not read stands as a newer major's.
///
/// # Errors
///
/// SQLite's, when the table cannot be read.
pub(crate) fn read_sqlite(connection: &Connection, name: &str) -> rusqlite::Result<Option<Marker>> {
    connection
        .prepare_cached("SELECT version, lowest_minor FROM format WHERE name = ?1")?
        .query_row([name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()
        .map(|found| {
            found.map(|(version, lowest_minor)| Marker {
                version: version.parse().unwrap_or(ProtocolVersion {
                    major: u16::MAX,
                    minor: 0,
                }),
                lowest_minor: u16::try_from(lowest_minor).unwrap_or(u16::MAX),
            })
        })
}

/// Records `marker` as the store's of format `name`.
///
/// # Errors
///
/// SQLite's, when the table cannot be written.
pub(crate) fn write_sqlite(
    connection: &Connection,
    name: &str,
    marker: Marker,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO format (name, version, lowest_minor) VALUES (?1, ?2, ?3)
         ON CONFLICT (name) DO UPDATE SET version = excluded.version, lowest_minor = excluded.lowest_minor",
        )?
        .execute((name, marker.version.to_string(), i64::from(marker.lowest_minor)))?;
    Ok(())
}

#[cfg(test)]
mod tests;
