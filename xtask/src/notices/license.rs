//! Judges a stated SPDX license expression against ADR 0018's allowlist for
//! one use, and names the license the inventory records.
//!
//! - An `OR` is allowed when one of its options is, the first such option
//!   chosen; an `AND` when all its parts are. Cargo's legacy `/` is an `OR`.
//! - An MPL-2.0 file must be unmodified (ADR 0018); the allowlist's `only`
//!   says which licenses are allowed for one use or one package alone.
//! - The compiled form of a Filter list is an adaptation, recorded under the
//!   allowlist's CC BY-SA: a CC BY-SA option of the list's, or a CC BY one,
//!   the stricter CC BY-SA then (ADR 0047). CC BY 3.0 and later allow any
//!   license for an adaptation, and CC BY-SA 3.0 and later a later version.
//! - The GPL family is allowed only for an input that is downloaded, never
//!   embedded (ADRs 0018 and 0047).

use std::path::Path;

use anyhow::Result;

use super::entry::{Ecosystem, Entry, Usage};
use crate::license_allowlist::{self, Only, PackageEcosystem, Use};

/// The license under which an embedded, compiled Filter list is recorded.
const COMPILED_FILTER_LISTS: &str = "CC-BY-SA-4.0";

/// The licenses whose lists may be adapted into `COMPILED_FILTER_LISTS`.
const ADAPTABLE: [&str; 4] = ["CC-BY-3.0", "CC-BY-4.0", "CC-BY-SA-3.0", "CC-BY-SA-4.0"];

/// The GPL family's identifiers begin with one of these.
const GPL_FAMILY: [&str; 3] = ["GPL-", "LGPL-", "AGPL-"];

pub(crate) struct Allowlist {
    licenses: Vec<license_allowlist::License>,
}

#[derive(PartialEq, Eq, Debug)]
pub(crate) enum Verdict {
    /// Allowed, recorded under this license.
    Allowed(String),
    Refused(Refusal),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Refusal {
    /// No license is stated, or the statement is no SPDX expression.
    Unknown,
    /// A license outside the allowlist.
    NotAllowed,
    /// A GPL license on something embedded.
    GplEmbedded,
    /// A license allowed only for unmodified files, on a modified one.
    Modified,
}

impl Refusal {
    /// The finding's kind.
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Unknown => "license-unknown",
            Self::NotAllowed => "license-not-allowed",
            Self::GplEmbedded => "gpl-embedded",
            Self::Modified => "license-modified-file",
        }
    }

    pub(crate) fn message(self, stated: Option<&str>) -> String {
        let stated = stated.unwrap_or("none");
        match self {
            Self::Unknown => format!(
                "its license ({stated}) is no SPDX expression the check can read; state one"
            ),
            Self::NotAllowed => format!(
                "its license ({stated}) is not on ADR 0018's allowlist in {}",
                license_allowlist::PATH
            ),
            Self::GplEmbedded => format!(
                "its license ({stated}) allows it to be downloaded only, never embedded (ADRs 0018 and 0047)"
            ),
            Self::Modified => format!(
                "its license ({stated}) is allowed for unmodified files only, and it is modified (ADR 0018)"
            ),
        }
    }
}

impl Allowlist {
    pub(crate) fn read(root: &Path) -> Result<Self> {
        Ok(Self {
            licenses: license_allowlist::read(root)?,
        })
    }

    /// The verdict on the license `entry` states, for its use.
    pub(crate) fn judge(&self, entry: &Entry) -> Verdict {
        let (stated, usage) = (entry.license.as_deref(), entry.usage);
        let Some(options) = stated.and_then(options) else {
            return Verdict::Refused(Refusal::Unknown);
        };
        let mut first_refusal = None;
        for option in &options {
            match option
                .iter()
                .map(|id| self.leaf(id, entry))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(recorded) if usage == Usage::CompiledFilterList => {
                    return Verdict::Allowed(recorded.join(" AND "));
                }
                Ok(_) => {
                    return Verdict::Allowed(stated.unwrap_or_default().trim().to_owned());
                }
                Err(refusal) => {
                    first_refusal.get_or_insert(refusal);
                }
            }
        }
        Verdict::Refused(first_refusal.unwrap_or(Refusal::Unknown))
    }

    /// The license one identifier is recorded under, or why it is refused.
    fn leaf(&self, id: &str, entry: &Entry) -> Result<String, Refusal> {
        let usage = entry.usage;
        let id = if usage == Usage::CompiledFilterList
            && ADAPTABLE.iter().any(|license| same(license, id))
        {
            COMPILED_FILTER_LISTS
        } else {
            id
        };
        let mut entries = self
            .licenses
            .iter()
            .filter(|license| same(&license.id, id))
            .peekable();
        if entries.peek().is_none() {
            let base = id.split_whitespace().next().unwrap_or_default();
            let gpl = GPL_FAMILY.iter().any(|prefix| {
                base.get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            });
            return match (gpl, usage) {
                (true, Usage::DownloadOnly) => Ok(id.to_owned()),
                (true, _) => Err(Refusal::GplEmbedded),
                (false, _) => Err(Refusal::NotAllowed),
            };
        }
        // A license allowed for several packages has one entry each: the
        // first that allows this use decides.
        let mut first_refusal = None;
        for license in entries {
            match allows(license, entry) {
                Ok(recorded) => return Ok(recorded),
                Err(refusal) => {
                    first_refusal.get_or_insert(refusal);
                }
            }
        }
        Err(first_refusal.unwrap_or(Refusal::NotAllowed))
    }
}

/// The license `license` records `entry` under, or why that entry of the
/// allowlist refuses its use.
fn allows(license: &license_allowlist::License, entry: &Entry) -> Result<String, Refusal> {
    match (&license.only, entry.usage) {
        (Some(Only::Use(Use::UnmodifiedFiles)), Usage::Modified) => Err(Refusal::Modified),
        (Some(Only::Use(Use::CompiledFilterLists)), Usage::Unmodified | Usage::Modified) => {
            Err(Refusal::NotAllowed)
        }
        (Some(Only::Package(package)), _) => {
            let ecosystem = match package.ecosystem {
                PackageEcosystem::Npm => Ecosystem::Npm,
                PackageEcosystem::Python => Ecosystem::Python,
            };
            if entry.ecosystem == ecosystem && entry.name == package.package {
                Ok(license.id.clone())
            } else {
                Err(Refusal::NotAllowed)
            }
        }
        _ => Ok(license.id.clone()),
    }
}

/// Two identifiers name one license: SPDX compares them ignoring case.
fn same(left: &str, right: &str) -> bool {
    left.split_whitespace()
        .map(str::to_ascii_lowercase)
        .eq(right.split_whitespace().map(str::to_ascii_lowercase))
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Token {
    Open,
    Close,
    Or,
    And,
    With,
    Id(String),
}

/// Whether `text` is an SPDX expression the check can read.
pub(super) fn readable(text: &str) -> bool {
    options(text).is_some()
}

/// The expression's options, each the identifiers all of which apply (its
/// disjunctive normal form); `None` when it is no expression.
fn options(text: &str) -> Option<Vec<Vec<String>>> {
    let mut parser = Parser {
        tokens: tokens(text),
        position: 0,
    };
    let options = parser.or()?;
    (parser.position == parser.tokens.len()).then_some(options)
}

fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    for character in text.chars() {
        let token = match character {
            '(' => Some(Token::Open),
            ')' => Some(Token::Close),
            '/' => Some(Token::Or),
            _ if character.is_whitespace() => None,
            _ => {
                word.push(character);
                continue;
            }
        };
        end_word(&mut word, &mut tokens);
        tokens.extend(token);
    }
    end_word(&mut word, &mut tokens);
    tokens
}

/// Ends the word being read, if any, as an operator or an identifier.
fn end_word(word: &mut String, tokens: &mut Vec<Token>) {
    if word.is_empty() {
        return;
    }
    tokens.push(match word.to_ascii_uppercase().as_str() {
        "OR" => Token::Or,
        "AND" => Token::And,
        "WITH" => Token::With,
        _ => Token::Id(std::mem::take(word)),
    });
    word.clear();
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn next_is(&self, token: &Token) -> bool {
        self.tokens.get(self.position) == Some(token)
    }

    fn or(&mut self) -> Option<Vec<Vec<String>>> {
        let mut options = self.and()?;
        while self.next_is(&Token::Or) {
            self.position += 1;
            options.extend(self.and()?);
        }
        Some(options)
    }

    fn and(&mut self) -> Option<Vec<Vec<String>>> {
        let mut options = self.term()?;
        while self.next_is(&Token::And) {
            self.position += 1;
            let right = self.term()?;
            options = options
                .iter()
                .flat_map(|left| {
                    right
                        .iter()
                        .map(move |right| left.iter().chain(right).cloned().collect())
                })
                .collect();
        }
        Some(options)
    }

    fn term(&mut self) -> Option<Vec<Vec<String>>> {
        match self.tokens.get(self.position)?.clone() {
            Token::Open => {
                self.position += 1;
                let inner = self.or()?;
                self.next_is(&Token::Close).then(|| {
                    self.position += 1;
                    inner
                })
            }
            Token::Id(id) => {
                self.position += 1;
                if self.next_is(&Token::With) {
                    let Some(Token::Id(exception)) = self.tokens.get(self.position + 1).cloned()
                    else {
                        return None;
                    };
                    self.position += 2;
                    return Some(vec![vec![format!("{id} WITH {exception}")]]);
                }
                Some(vec![vec![id]])
            }
            Token::Close | Token::Or | Token::And | Token::With => None,
        }
    }
}

#[cfg(test)]
mod tests;
