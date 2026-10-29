//! Trap links (ADR 0035; pitfall S03; spec #600): on the browser tier a
//! crawl never follows a link a person cannot see. [`READ`] asks Collie's
//! page, in the guard's isolated world, what a person sees of each element
//! a locator matched: whether it has a box, lies inside the page, is hidden
//! by `visibility` or by an `opacity` of 0 on it or an element holding it,
//! sits in an `aria-hidden` subtree or is clipped to nothing, whether it
//! carries `rel="nofollow"`, and whether an element holding it is one a
//! cosmetic Clean-up rule matched (ADR 0047). [`judge`] decides from that:
//! a link a cosmetic rule hides is left out of the judgement, so a link in
//! a hidden ad is no trap; any other invisible link, `nofollow` or not, is
//! a trap, answered "not added" with `trap_link`.

use encoding_rs::Encoding;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::Value;
use url::Url;

use crate::identity::{Declared, Key, Seen as Visited, identities, seen};
use crate::url::parse_link;

/// The page function that reads what a person sees of each element a
/// selector matches, called with `{ selector, cosmetic }`, `cosmetic` the
/// selectors of the cosmetic rules the page's live hiding applies.
pub const READ: &str = include_str!("trap_links/read.js");

/// What a person sees of one matched element, as [`READ`] reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one independent fact the page reported"
)]
pub struct Seen {
    /// It has no box: `display: none`, `hidden`, or no area.
    pub no_box: bool,
    /// Its box lies wholly outside the page.
    pub outside_page: bool,
    /// Its computed `visibility` is not `visible`.
    pub visibility_hidden: bool,
    /// It, or an element holding it, has an `opacity` of 0.
    pub transparent: bool,
    /// It sits in an `aria-hidden="true"` subtree.
    pub aria_hidden: bool,
    /// An element holding it clips it to nothing.
    pub clipped: bool,
    /// It carries `rel="nofollow"`.
    pub nofollow: bool,
    /// It, or an element holding it, is one a cosmetic rule matched.
    pub cosmetic: bool,
}

impl Seen {
    /// The facts [`READ`] answered for one element; `None` for a value it
    /// never answers.
    #[must_use]
    pub fn read(value: &Value) -> Option<Self> {
        let fact = |name: &str| value.get(name).and_then(Value::as_bool);
        Some(Self {
            no_box: fact("noBox")?,
            outside_page: fact("outsidePage")?,
            visibility_hidden: fact("visibilityHidden")?,
            transparent: fact("transparent")?,
            aria_hidden: fact("ariaHidden")?,
            clipped: fact("clipped")?,
            nofollow: fact("nofollow")?,
            cosmetic: fact("cosmetic")?,
        })
    }

    /// What a served document's markup alone states of a link, from the
    /// attributes of each of `elements`, the link first, then each element
    /// holding it outward: a `hidden` attribute or an inline
    /// `display: none`, an inline `visibility` other than `visible`, an
    /// inline `opacity` of 0, `aria-hidden="true"`, and the link's own
    /// `rel="nofollow"`. A document fetched over HTTP has no layout, so
    /// what only layout or a style sheet decides reads as seen.
    #[must_use]
    pub fn in_markup<'a>(elements: impl IntoIterator<Item = &'a [(String, String)]>) -> Self {
        let mut seen = Self::default();
        for (at, attributes) in elements.into_iter().enumerate() {
            let value = |name: &str| {
                attributes
                    .iter()
                    .find(|(attribute, _)| attribute.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.as_str())
            };
            let style = value("style").map(inline_style).unwrap_or_default();
            let declared = |property: &str| {
                style
                    .iter()
                    .find(|(name, _)| name == property)
                    .map(|(_, value)| value.as_str())
            };
            seen.no_box |= value("hidden").is_some() || declared("display") == Some("none");
            seen.visibility_hidden |=
                declared("visibility").is_some_and(|visibility| visibility != "visible");
            seen.transparent |= declared("opacity")
                .and_then(|opacity| opacity.parse::<f64>().ok())
                .is_some_and(|opacity| opacity <= 0.0);
            seen.aria_hidden |= value("aria-hidden")
                .is_some_and(|hidden| hidden.trim().eq_ignore_ascii_case("true"));
            if at == 0 {
                seen.nofollow = value("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("nofollow"))
                });
            }
        }
        seen
    }
}

/// The declarations of an inline `style` attribute, each property and
/// value lower-cased and trimmed, `!important` dropped.
fn inline_style(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let value = value.trim().to_ascii_lowercase();
            let value = value.strip_suffix("!important").unwrap_or(&value).trim();
            Some((property.trim().to_ascii_lowercase(), value.to_owned()))
        })
        .collect()
}

/// What a link's facts were read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    /// A browser page's layout, as [`READ`] reads it: a trap [`judge`]
    /// finds there is one.
    Layout(Seen),
    /// A served document's markup, as [`Seen::in_markup`] reads it, which
    /// no layout confirms: a trap [`judge`] finds there is only suspected.
    Markup(Seen),
}

/// The Provenance action that marks a link added though its served
/// document's markup hides it, a suspected trap.
pub const SUSPECTED_ACTION: &str = "trap_link_suspected";

/// The table of the work items added through a link suspected a trap,
/// which a browser tier never follows.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-trap-links",
    statements: &["CREATE TABLE IF NOT EXISTS trap_link_suspected (
            work_item TEXT NOT NULL PRIMARY KEY
        ) STRICT, WITHOUT ROWID"],
};

/// Marks the work item `id` as added through a link suspected a trap.
///
/// # Errors
///
/// The run state's.
pub fn suspect(connection: &Connection, id: &str) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO trap_link_suspected (work_item) VALUES (?1)",
        [id],
    )?;
    Ok(())
}

/// Clears the mark of the work item that the link `href`, resolved
/// against `base` with its query in `encoding`, reaches again, once the
/// Frontier answered it `duplicate`, when the link is one a person sees or
/// whose markup hides nothing: the request the visited set names for it,
/// or the one a learned rule merged it into, is then reached through a
/// link that is not suspected, so it is no suspected trap (ADR 0035's
/// #1153 note, as finding 600-1 scopes it). Run inside the admission that
/// answered the link, so the clear commits or is discarded with it. A link
/// to a URL the Frontier never admitted clears nothing.
///
/// # Errors
///
/// The run state's.
pub fn seen_again(
    connection: &Connection,
    href: &str,
    base: &Url,
    encoding: &'static Encoding,
    keeps_fragment: bool,
) -> rusqlite::Result<()> {
    let any_marked = connection
        .query_row("SELECT 1 FROM trap_link_suspected LIMIT 1", [], |_| Ok(()))
        .optional()?
        .is_some();
    if !any_marked {
        return Ok(());
    }
    let Ok(url) = parse_link(href, base, encoding) else {
        return Ok(());
    };
    let method = HttpMethod::get();
    let declared = Declared {
        method: &method,
        body: None,
        key: None,
        keeps_fragment,
    };
    let identities = identities(connection, &Key::of(&url, keeps_fragment), declared)?;
    let reached = match seen(connection, &identities)? {
        Visited::New => return Ok(()),
        Visited::Visited(crawl_request) | Visited::Merged { crawl_request, .. } => crawl_request,
    };
    connection.execute(
        "DELETE FROM trap_link_suspected WHERE work_item = ?1",
        [reached],
    )?;
    Ok(())
}

/// Whether the work item `id` was added through a link suspected a trap.
///
/// # Errors
///
/// The run state's.
pub fn suspected(connection: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM trap_link_suspected WHERE work_item = ?1",
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Why a link is a trap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    /// A person cannot see it.
    Invisible,
    /// A person cannot see it, and it carries `rel="nofollow"`.
    InvisibleNofollow,
}

impl Trap {
    /// Its name, for traces.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Invisible => "invisible",
            Self::InvisibleNofollow => "invisible_nofollow",
        }
    }
}

/// Whether the link `seen` describes is a trap: `None` for one a person
/// sees, or one a cosmetic rule hides, which is left out of the judgement.
#[must_use]
pub fn judge(seen: &Seen) -> Option<Trap> {
    let Seen {
        no_box,
        outside_page,
        visibility_hidden,
        transparent,
        aria_hidden,
        clipped,
        nofollow,
        cosmetic,
    } = *seen;
    let invisible =
        no_box || outside_page || visibility_hidden || transparent || aria_hidden || clipped;
    match (cosmetic, invisible, nofollow) {
        (true, _, _) | (false, false, _) => None,
        (false, true, false) => Some(Trap::Invisible),
        (false, true, true) => Some(Trap::InvisibleNofollow),
    }
}

#[cfg(test)]
mod tests;
