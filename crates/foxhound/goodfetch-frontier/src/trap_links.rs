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

use serde_json::Value;

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
