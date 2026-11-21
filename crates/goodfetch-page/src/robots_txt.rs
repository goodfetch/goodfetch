//! robots.txt's group selection (RFC 9309 §2.2.1; ADRs 0086 and 0090):
//! which of a file's groups a crawl obeys. A group applies when one of its
//! `User-agent` lines names `goodfetch-foxhound` or the operator's own
//! product token (`site_preferences.robots_txt_user_agent`), compared as a
//! whole product token in any case and never as a substring, so a bare
//! `Foxhound` names neither; the groups that apply merge into one, and only
//! when none does the `*` groups apply. The selection is the same in every
//! Access mode. Area 6's reading of robots.txt for a Collie program's own
//! navigation and Foxhound's navigation gate both call it; parsing the file
//! and matching a path against the rules are theirs.

/// A robots.txt product token: letters, `_` and `-` (RFC 9309 §2.2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductToken(String);

impl ProductToken {
    /// Foxhound's own, `goodfetch-foxhound` (ADR 0090).
    pub fn foxhound() -> Self {
        Self("goodfetch-foxhound".to_owned())
    }

    /// `token`, when it is one.
    ///
    /// # Errors
    ///
    /// [`InvalidProductToken`] when it is empty or holds a character other
    /// than an ASCII letter, `_` or `-`.
    pub fn new(token: &str) -> Result<Self, InvalidProductToken> {
        if !token.is_empty() && token.chars().all(is_token_character) {
            Ok(Self(token.to_owned()))
        } else {
            Err(InvalidProductToken(token.to_owned()))
        }
    }

    /// Whether a `User-agent` line's value names this token: its leading
    /// product token, up to the first character no token holds (such as
    /// `/1.2`), equals this one in any case, as Google's reference parser
    /// reads RFC 9309.
    fn named_by(&self, user_agent: &str) -> bool {
        let user_agent = user_agent.trim();
        let end = user_agent
            .find(|character: char| !is_token_character(character))
            .unwrap_or(user_agent.len());
        user_agent[..end].eq_ignore_ascii_case(&self.0)
    }

    /// The token as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A product token with a character RFC 9309 does not allow in one.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is no robots.txt product token: use only letters, `_` and `-`")]
#[non_exhaustive]
pub struct InvalidProductToken(String);

/// One group of a robots.txt file: its `User-agent` lines' values, as
/// written, and its rules in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<R> {
    /// The values of its `User-agent` lines.
    pub user_agents: Vec<String>,
    /// Its rules.
    pub rules: Vec<R>,
}

/// The rules a crawl obeys from `groups`, a robots.txt file's groups in file
/// order: those of every group that names `goodfetch-foxhound` or
/// `operator`, merged in file order; when none does, those of every `*`
/// group; when there is none either, no rule.
pub fn applicable_rules<'a, R>(
    groups: &'a [Group<R>],
    operator: Option<&ProductToken>,
) -> Vec<&'a R> {
    applicable_groups(groups, |group| &group.user_agents, operator)
        .into_iter()
        .flat_map(|group| &group.rules)
        .collect()
}

/// The groups a crawl obeys of `groups`, a robots.txt file's groups in
/// file order, each read for its `User-agent` lines' values with
/// `user_agents`, as [`applicable_rules`] selects them, in file order: a
/// parse's own groups are selected without copying their values (#1899).
pub fn applicable_groups<'a, G>(
    groups: &'a [G],
    user_agents: impl Fn(&G) -> &[String],
    operator: Option<&ProductToken>,
) -> Vec<&'a G> {
    let foxhound = ProductToken::foxhound();
    let named = |group: &G| {
        user_agents(group).iter().any(|user_agent| {
            foxhound.named_by(user_agent)
                || operator.is_some_and(|token| token.named_by(user_agent))
        })
    };
    let everyone = |group: &G| {
        user_agents(group)
            .iter()
            .any(|user_agent| is_star(user_agent))
    };
    let any_named = groups.iter().any(named);
    groups
        .iter()
        .filter(|group| {
            if any_named {
                named(group)
            } else {
                everyone(group)
            }
        })
        .collect()
}

/// Whether RFC 9309 allows `character` in a product token.
fn is_token_character(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_' || character == '-'
}

/// Whether a `User-agent` line's value is `*`, alone or before a space, as
/// Google's reference parser reads it.
fn is_star(user_agent: &str) -> bool {
    user_agent
        .trim()
        .strip_prefix('*')
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}
