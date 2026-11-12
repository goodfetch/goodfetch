//! A citation read against the repository: whether its path, symbol and
//! lines are on `main` as they were at the list's read commit, and, when they
//! are not, the first merge of `main` after that commit that moved them.

use std::path::Path;

use anyhow::Result;

use super::list::Citation;
use crate::git;

/// The repository a list is checked in, and the commit of `main` it is
/// checked against.
pub(super) struct Repository<'a> {
    pub(super) root: &'a Path,
    pub(super) main: String,
}

/// A citation that holds on `main`.
pub(super) enum Holds {
    Unchanged,
    /// Its lines are on `main` as they were, from `first` to `last`.
    Moved {
        first: usize,
        last: usize,
    },
}

/// Why a citation does not hold.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Broken {
    MissingPath,
    MissingSymbol,
    /// Its lines are past the end of its file at the read commit, or the path
    /// is no file there: the citation never named them.
    UnknownLines,
    ChangedLines,
}

impl Repository<'_> {
    /// Whether `path` is a file or folder at `revision`.
    pub(super) fn exists(&self, revision: &str, path: &str) -> Result<bool> {
        Ok(git::succeeds(
            self.root,
            &["cat-file", "-e", &format!("{revision}:{path}")],
        )?)
    }

    /// The file at `path` of `revision` as text, or `None` when it is no
    /// file there.
    pub(super) fn file(&self, revision: &str, path: &str) -> Result<Option<String>> {
        match git::bytes(
            self.root,
            &["cat-file", "blob", &format!("{revision}:{path}")],
        ) {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(git::Error::Refused { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Whether `citation`, read at `read`, holds at `revision`.
    pub(super) fn holds(
        &self,
        citation: &Citation,
        read: &str,
        revision: &str,
    ) -> Result<Result<Holds, Broken>> {
        if !self.exists(revision, &citation.path)? {
            return Ok(Err(Broken::MissingPath));
        }
        let file = self.file(revision, &citation.path)?;
        if let Some(symbol) = &citation.symbol
            && !file.as_deref().is_some_and(|text| names(text, symbol))
        {
            return Ok(Err(Broken::MissingSymbol));
        }
        let Some((first, last)) = citation.lines else {
            return Ok(Ok(Holds::Unchanged));
        };
        let cited = self.file(read, &citation.path)?;
        let Some(cited) = cited.as_deref().and_then(|text| {
            text.lines()
                .collect::<Vec<_>>()
                .get(first - 1..last)
                .map(<[_]>::to_vec)
        }) else {
            return Ok(Err(Broken::UnknownLines));
        };
        let now: Vec<&str> = file.as_deref().unwrap_or_default().lines().collect();
        if now.get(first - 1..last) == Some(cited.as_slice()) {
            return Ok(Ok(Holds::Unchanged));
        }
        let nearest = now
            .windows(cited.len())
            .enumerate()
            .filter(|(_, window)| *window == cited.as_slice())
            .map(|(start, _)| start)
            .min_by_key(|start| start.abs_diff(first - 1));
        Ok(match nearest {
            Some(start) => Ok(Holds::Moved {
                first: start + 1,
                last: start + cited.len(),
            }),
            None => Err(Broken::ChangedLines),
        })
    }

    /// The first commit of `main`'s first-parent line after `read` at which
    /// `citation` no longer holds, as `<9 digits> (<subject>)`: the merge
    /// that moved its evidence. `None` when no merge since touched its path:
    /// it did not hold at `read` either.
    pub(super) fn moved_by(&self, citation: &Citation, read: &str) -> Result<Option<String>> {
        let log = git::text(
            self.root,
            &[
                "log",
                "--first-parent",
                "--reverse",
                "--format=%H %s",
                &format!("{read}..{}", self.main),
                "--",
                &citation.path,
            ],
        )?;
        for line in log.lines() {
            let (commit, subject) = line.split_once(' ').unwrap_or((line, ""));
            if self.holds(citation, read, commit)?.is_err() {
                let short = commit.get(..9).unwrap_or(commit);
                return Ok(Some(format!("{short} ({subject})")));
            }
        }
        Ok(None)
    }
}

/// Whether `text` names `symbol`: each of its parts, split at `::` and `.`
/// (`Reader::of`, `dialogs.close`), as a whole word: a part that starts or
/// ends with a letter, digit or `_` has none of them right before or after it
/// there, so `keep` is not found in `keeper`, while `analysis(` is found in
/// `analysis(&page)`.
fn names(text: &str, symbol: &str) -> bool {
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    let word = |character: Option<char>| character.is_some_and(is_word);
    let whole_word = |part: &str| {
        let starts_a_word = part.chars().next().is_some_and(is_word);
        let ends_a_word = part.chars().next_back().is_some_and(is_word);
        text.match_indices(part).any(|(start, found)| {
            let joined_before = starts_a_word && word(text[..start].chars().next_back());
            let joined_after = ends_a_word && word(text[start + found.len()..].chars().next());
            !joined_before && !joined_after
        })
    };
    symbol
        .split("::")
        .flat_map(|part| part.split('.'))
        .filter(|part| !part.is_empty())
        .all(whole_word)
}

#[cfg(test)]
mod tests {
    use super::names;

    #[test]
    fn a_symbol_is_named_as_a_whole_word() {
        assert!(names("pub(crate) fn keep(", "fn keep"));
        assert!(names("# `just gate-backstop [ref]`", "gate-backstop"));
        assert!(!names("fn keeper()", "keep"));
        assert!(!names("fn _keep()", "keep"));
        assert!(names("keep", "keep"));
        assert!(names("impl Reader {\n    fn of(", "Reader::of"));
        assert!(names("self.dialogs.close();", "dialogs.close"));
        assert!(!names("impl Reader {\n    fn off(", "Reader::of"));
        assert!(names("tier.analysis(&page)", ".analysis("));
        assert!(names(
            "host: self.host_defaults(),",
            "host: self.host_defaults()"
        ));
        assert!(!names("tier.reanalysis(&page)", ".analysis("));
    }
}
