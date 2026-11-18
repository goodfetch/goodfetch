//! The `skill:` Features: one per agent skill, a folder of `skills/` that
//! holds a `SKILL.md` (the Agent Skills format), named by its folder. Collie's
//! topics ship with both Products and Foxhound's, under `skills/foxhound/`,
//! with Foxhound alone (ADR 0095). A folder without a `SKILL.md`, and a
//! workspace without `skills/`, generate nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context as _, Result};

use super::entry::{Carriers, Entry};
use super::files::{Kind, Status};

/// The folder of the skills' sources.
const SKILLS: &str = "skills";
/// The folder, in [`SKILLS`], of Foxhound's own topics.
const FOXHOUND: &str = "foxhound";
/// The file that makes a folder a skill.
const SKILL_FILE: &str = "SKILL.md";

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let skills = root.join(SKILLS);
    let mut entries = skills_in(&skills, Carriers::Both)?;
    entries.extend(skills_in(&skills.join(FOXHOUND), Carriers::Foxhound)?);
    Ok(entries)
}

/// The skills directly in `folder`, carried by `carriers`.
fn skills_in(folder: &Path, carriers: Carriers) -> Result<Vec<Entry>> {
    let listing = match std::fs::read_dir(folder) {
        Ok(listing) => listing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("listing {}", folder.display()));
        }
    };
    let mut entries = Vec::new();
    for item in listing {
        let item = item.with_context(|| format!("listing {}", folder.display()))?;
        let is_folder = item
            .file_type()
            .with_context(|| format!("reading {}", item.path().display()))?
            .is_dir();
        if !is_folder {
            continue;
        }
        let skill_file = item.path().join(SKILL_FILE);
        let is_skill = skill_file
            .try_exists()
            .with_context(|| format!("reading {}", skill_file.display()))?;
        if !is_skill {
            continue;
        }
        let name = item.file_name().into_string().map_err(|name| {
            anyhow::anyhow!("{} is not named in UTF-8", Path::new(&name).display())
        })?;
        entries.push(Entry {
            kind: Kind::Skill,
            name,
            area: None,
            milestone: None,
            status: Status::Active,
            carriers,
        });
    }
    Ok(entries)
}
