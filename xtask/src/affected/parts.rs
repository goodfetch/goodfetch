//! The parts of a changed Markdown document, each named by its heading line
//! (`## Quality register`) and running from it to the next heading of its
//! level or above. A test that reads a document only in part finds each part
//! by its heading line, which its member's code then names in a string
//! literal, so a change to the other parts reaches none of its tests: a
//! prose edit of `TESTING.md` takes the docs gate (hygiene backlog E1).

use std::collections::{BTreeMap, BTreeSet};

use super::RepoPath;

/// The changed Markdown documents whose base and head versions were both
/// read, with their parts. A document without an entry is taken as changed
/// throughout, as is any other changed file.
#[derive(Default)]
pub(crate) struct ChangedParts(BTreeMap<RepoPath, Parts>);

struct Parts {
    /// Every heading line of either version.
    headings: BTreeSet<String>,
    /// The heading lines whose part differs between the versions, or is in
    /// one only.
    changed: BTreeSet<String>,
}

impl ChangedParts {
    /// Records `path`'s parts from its base's text and its head's.
    pub(crate) fn insert(&mut self, path: RepoPath, before: &str, after: &str) {
        let before = parts(before);
        let after = parts(after);
        let headings: BTreeSet<String> = before.keys().chain(after.keys()).cloned().collect();
        let changed = headings
            .iter()
            .filter(|heading| before.get(*heading) != after.get(*heading))
            .cloned()
            .collect();
        self.0.insert(path, Parts { headings, changed });
    }

    /// Every heading line of `path`, when its parts were read.
    pub(crate) fn headings(&self, path: &RepoPath) -> Option<&BTreeSet<String>> {
        self.0.get(path).map(|parts| &parts.headings)
    }

    /// The heading lines of `path`'s changed parts, when its parts were read.
    pub(crate) fn changed(&self, path: &RepoPath) -> Option<&BTreeSet<String>> {
        self.0.get(path).map(|parts| &parts.changed)
    }

    /// Whether the change reaches the part of `path` under `heading`: true
    /// when its parts were not read.
    pub(crate) fn reaches(&self, path: &RepoPath, heading: &str) -> bool {
        self.changed(path)
            .is_none_or(|changed| changed.contains(heading))
    }
}

/// Each heading line of `text` with the texts of its parts, in order: a
/// heading line may appear more than once.
fn parts(text: &str) -> BTreeMap<String, Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let headings = heading_lines(&lines);
    let mut parts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (position, &(start, level)) in headings.iter().enumerate() {
        let end = headings[position + 1..]
            .iter()
            .find(|(_, other)| *other <= level)
            .map_or(lines.len(), |(line, _)| *line);
        parts
            .entry(lines[start].trim().to_owned())
            .or_default()
            .push(lines[start..end].join("\n"));
    }
    parts
}

/// The index and level of each ATX heading line of `lines`, outside fenced
/// code blocks.
fn heading_lines(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut fence: Option<&str> = None;
    let mut headings = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if let Some(marker) = ["```", "~~~"]
            .into_iter()
            .find(|marker| trimmed.starts_with(marker))
        {
            fence = match fence {
                Some(open) if open == marker => None,
                Some(open) => Some(open),
                None => Some(marker),
            };
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let level = line
            .chars()
            .take_while(|character| *character == '#')
            .count();
        let titled = line[level..].starts_with(' ') || line.len() == level;
        if (1..=6).contains(&level) && titled {
            headings.push((index, level));
        }
    }
    headings
}
