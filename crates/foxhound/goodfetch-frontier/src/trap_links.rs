//! Trap links (ADR 0035; pitfall S03; spec #600): on the browser tier a
//! crawl never follows a link a person cannot see. [`READ`] asks Collie's
//! page, in the guard's isolated world, what a person sees of each element
//! a locator matched: whether it has a box, lies inside the page, is hidden
//! by `visibility` or by an `opacity` of 0 on it or an element holding it,
//! sits in an `aria-hidden` subtree or is clipped to nothing, whether it
//! carries `rel="nofollow"`, whether an element holding it is one a
//! cosmetic Clean-up rule matched (ADR 0047), and whether it sits in a
//! menu, a tab or a disclosure a person opens. [`judge`] decides from that:
//! a link a cosmetic rule hides is left out of the judgement, so a link in
//! a hidden ad is no trap; an invisible link in a menu, tab or disclosure
//! is one a person reveals, followed and marked as a suspected trap; any
//! other invisible link, and every invisible `nofollow` one, the signal
//! Cloudflare's AI Labyrinth gives, is a trap, answered "not added" with
//! `trap_link` (ADR 0035, owner decision O26).

use goodfetch_engine::run_state::RunStateTables;
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Value, json};

use crate::admit::Keyed;
use crate::identity::{Seen as Visited, seen, under_rules};

/// The page function that reads what a person sees of each element a
/// selector matches, called with [`read_params`] and `cosmetic`, the
/// selectors of the cosmetic rules the page's live hiding applies.
pub const READ: &str = include_str!("trap_links/read.js");

/// The elements whose content a person opens to see: a navigation menu,
/// a menu and a disclosure.
const OPENING_ELEMENTS: &[&str] = &["details", "menu", "nav"];

/// The roles of an element whose content a person opens to see, or of one
/// inside such an element: a navigation's, a menu's and a tab's
/// (WAI-ARIA).
const OPENING_ROLES: &[&str] = &[
    "menu",
    "menubar",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "navigation",
    "tab",
    "tablist",
    "tabpanel",
];

/// The argument [`READ`] reads the elements `selector` matches with, the
/// elements and roles that open as [`opens`] reads them included, so both
/// tiers read one list; the page adds `cosmetic`.
#[must_use]
pub fn read_params(selector: &str) -> Value {
    json!({
        "selector": selector,
        "opening": { "elements": OPENING_ELEMENTS, "roles": OPENING_ROLES },
    })
}

/// Whether the element `name`, lower-cased, with `attributes`, holds what
/// it holds in a menu, a tab or a disclosure a person opens to see: a
/// `nav`, `menu` or `details` element, an element with a menu, navigation
/// or tab role, a `popover`, a `hidden` that a find in the page opens
/// (`until-found`), or one another element names in `aria-controls`
/// (`controlled`), which a button or tab shows and hides. [`READ`] reads
/// the same on a browser page.
#[must_use]
pub fn opens(name: &str, attributes: &[(String, String)], controlled: bool) -> bool {
    let value = |name: &str| {
        attributes
            .iter()
            .find(|(attribute, _)| attribute.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    };
    controlled
        || OPENING_ELEMENTS.contains(&name)
        || value("popover").is_some()
        || value("hidden").is_some_and(|hidden| hidden.eq_ignore_ascii_case("until-found"))
        || value("role").is_some_and(|roles| {
            roles.split_ascii_whitespace().any(|role| {
                OPENING_ROLES
                    .iter()
                    .any(|opening| opening.eq_ignore_ascii_case(role))
            })
        })
}

/// Whether an `aria-hidden` value hides: `true`, past any ASCII whitespace,
/// in any ASCII case. [`READ`] reads it alike.
fn hides(aria_hidden: &str) -> bool {
    aria_hidden.trim_ascii().eq_ignore_ascii_case("true")
}

/// One element of a served document, as [`Seen::in_markup`] reads it.
#[derive(Clone, Copy, Debug)]
pub struct Markup<'a> {
    /// Its tag name, lower-cased.
    pub name: &'a str,
    /// Its attributes, in document order.
    pub attributes: &'a [(String, String)],
    /// Whether an element of its document names it in `aria-controls`.
    pub controlled: bool,
}

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
    /// It, or an element holding it, is a menu, a tab or a disclosure a
    /// person opens to see what it holds ([`opens`]).
    pub revealable: bool,
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
            revealable: fact("revealable")?,
        })
    }

    /// What a served document's markup alone states of a link, from the
    /// attributes of each of `elements`, the link first, then each element
    /// holding it outward: a `hidden` attribute or an inline
    /// `display: none`, an inline `visibility` other than `visible`, an
    /// inline `opacity` of 0, `aria-hidden="true"`, whether one of them is
    /// a menu, a tab or a disclosure a person opens ([`opens`]), and the
    /// link's own `rel="nofollow"`. A document fetched over HTTP has no
    /// layout, so what only layout or a style sheet decides reads as seen.
    #[must_use]
    pub fn in_markup<'a>(elements: impl IntoIterator<Item = Markup<'a>>) -> Self {
        let mut seen = Self::default();
        for (at, element) in elements.into_iter().enumerate() {
            let attributes = element.attributes;
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
            seen.aria_hidden |= value("aria-hidden").is_some_and(hides);
            seen.revealable |= opens(element.name, attributes, element.controlled);
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
/// each with whether a person reveals the link (`revealable`): a browser
/// tier never follows one whose markup alone hides it.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-trap-links",
    statements: &["CREATE TABLE IF NOT EXISTS trap_link_suspected (
            work_item TEXT NOT NULL PRIMARY KEY,
            revealable INTEGER NOT NULL
        ) STRICT, WITHOUT ROWID"],
};

/// How a link suspected a trap marked the work item it added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Its served document's markup hides it, which no layout confirms:
    /// a browser tier never follows it.
    Hidden,
    /// A menu, a tab or a disclosure a person opens hides it: it is
    /// followed on every tier.
    Revealable,
}

/// Marks the work item `id` as added through a link suspected a trap, as
/// `mark` says.
///
/// # Errors
///
/// The run state's.
pub fn suspect(connection: &Connection, id: &str, mark: Mark) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT OR IGNORE INTO trap_link_suspected (work_item, revealable) VALUES (?1, ?2)",
        )
        .and_then(|mut cached| cached.execute(rusqlite::params![id, mark == Mark::Revealable]))?;
    Ok(())
}

/// The statements [`seen_again`] runs, which [`crate::admission_statements`]
/// has the Run prepare as it starts: whether any request is marked, then a
/// mark's update to revealable or its removal.
pub(crate) const SEEN_AGAIN: [&str; 3] = [
    "SELECT 1 FROM trap_link_suspected LIMIT 1",
    "UPDATE trap_link_suspected SET revealable = 1 WHERE work_item = ?1",
    "DELETE FROM trap_link_suspected WHERE work_item = ?1",
];

/// Clears the mark of the work item that the link keyed as `keyed`
/// reaches again, once the Frontier answered it `duplicate`, when the link
/// is one a person sees or whose markup hides nothing: the request the
/// visited set names for it, or the one a learned rule merged it into, is
/// then reached through a link that is not suspected, so it is no
/// suspected trap (ADR 0035's #1153 note, as finding 600-1 scopes it).
/// When the link is one a person reveals (`revealable`), the mark stays,
/// as [`Mark::Revealable`]: the request is reached through a link a person
/// opens, which a browser tier follows. Run inside the admission that
/// answered the link, so the clear commits or is discarded with it. A link
/// to a URL the Frontier never admitted clears nothing.
///
/// # Errors
///
/// The run state's.
pub fn seen_again(
    connection: &Connection,
    keyed: &Keyed,
    revealable: bool,
) -> rusqlite::Result<()> {
    let any_marked = connection
        .prepare_cached(SEEN_AGAIN[0])
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some();
    if !any_marked {
        return Ok(());
    }
    let Some((key, fixed)) = keyed.identity() else {
        return Ok(());
    };
    let identities = under_rules(connection, key, fixed)?;
    // The run file only: a request an attempt staged holds no mark until
    // its commit, which admits this link again in order on the run file.
    let reached = match seen(connection, &identities, None)? {
        Visited::New => return Ok(()),
        Visited::Visited(crawl_request) | Visited::Merged { crawl_request, .. } => crawl_request,
    };
    if revealable {
        connection
            .prepare_cached(SEEN_AGAIN[1])
            .and_then(|mut cached| cached.execute([reached]))?;
    } else {
        connection
            .prepare_cached(SEEN_AGAIN[2])
            .and_then(|mut cached| cached.execute([reached]))?;
    }
    Ok(())
}

/// How the work item `id` is marked as added through a link suspected a
/// trap; `None` for one that is not.
///
/// # Errors
///
/// The run state's.
pub fn suspected(connection: &Connection, id: &str) -> rusqlite::Result<Option<Mark>> {
    connection
        .prepare_cached("SELECT revealable FROM trap_link_suspected WHERE work_item = ?1")
        .and_then(|mut cached| {
            cached.query_row([id], |row| {
                Ok(if row.get::<_, bool>(0)? {
                    Mark::Revealable
                } else {
                    Mark::Hidden
                })
            })
        })
        .optional()
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

/// What [`judge`] makes of a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A person sees it, or a cosmetic rule hides it, which leaves it out
    /// of the judgement.
    Seen,
    /// A person sees it once they open the menu, tab or disclosure holding
    /// it: it is followed, marked as a suspected trap.
    Revealable,
    /// A trap.
    Trap(Trap),
}

/// What the link `seen` describes is: one a person sees, or one a
/// cosmetic rule hides, which is left out of the judgement; one hidden in
/// a menu, a tab or a disclosure a person opens, unless it is `nofollow`;
/// or a trap.
#[must_use]
pub fn judge(seen: &Seen) -> Verdict {
    let Seen {
        no_box,
        outside_page,
        visibility_hidden,
        transparent,
        aria_hidden,
        clipped,
        nofollow,
        cosmetic,
        revealable,
    } = *seen;
    let invisible =
        no_box || outside_page || visibility_hidden || transparent || aria_hidden || clipped;
    match (cosmetic, invisible, nofollow, revealable) {
        (true, _, _, _) | (false, false, _, _) => Verdict::Seen,
        (false, true, false, true) => Verdict::Revealable,
        (false, true, false, false) => Verdict::Trap(Trap::Invisible),
        (false, true, true, _) => Verdict::Trap(Trap::InvisibleNofollow),
    }
}

#[cfg(test)]
mod tests;
