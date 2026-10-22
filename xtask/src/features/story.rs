//! The `story:` Features: each story of `features/stories/<area>.toml`, as
//! `story:<area>-<n>`, with its milestone, its status and, for a story of
//! Foxhound's part, the one Product that carries it.

use std::path::Path;

use anyhow::{Result, bail};

use super::entry::{Carriers, Entry, unreadable};
use super::files::{self, Kind, OnlyProduct, StoryFile};

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for (area, path) in files::story_files(root)? {
        let Some(area) = area else {
            bail!("{path} is not named by its area's number");
        };
        let file: StoryFile =
            files::read_toml(root, &path).map_err(|error| unreadable(error, &path))?;
        entries.extend(file.story.into_iter().map(|story| Entry {
            kind: Kind::Story,
            name: format!("{area}-{}", story.id),
            area: Some(area),
            milestone: Some(story.milestone),
            status: story.status,
            carriers: match story.product {
                Some(OnlyProduct::Foxhound) => Carriers::Foxhound,
                None => Carriers::Both,
            },
        }));
    }
    Ok(entries)
}
