//! The embedded Filter lists, one by one, and sets of them: what the other
//! modules name a list by, and credit a rule to.

/// One Filter list (built-in or from `filter_lists.extra`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum List {
    /// EasyList (`filter_lists.ads`).
    Ads,
    /// EasyPrivacy (`filter_lists.trackers`).
    Trackers,
    /// Fanboy Annoyance (`filter_lists.annoyances`).
    Annoyances,
    /// User-configured extra Filter lists (`filter_lists.extra`).
    Extra,
}

impl List {
    /// Its name, the last part of the `filter_lists.*` setting that turns
    /// it on, as Provenance names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ads => "ads",
            Self::Trackers => "trackers",
            Self::Annoyances => "annoyances",
            Self::Extra => "extra",
        }
    }
}

/// The three built-in lists, in the order a selector held by several is
/// credited to the first.
pub(crate) const LISTS: [List; 3] = [List::Ads, List::Trackers, List::Annoyances];

/// Every list, built-in first and `filter_lists.extra` after.
pub(crate) const ALL_LISTS: [List; 4] = [List::Ads, List::Trackers, List::Annoyances, List::Extra];

/// A set of the Filter lists, one bit each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct ListSet(u8);

impl ListSet {
    fn bit(list: List) -> u8 {
        match list {
            List::Ads => 1,
            List::Trackers => 2,
            List::Annoyances => 4,
            List::Extra => 8,
        }
    }

    pub(crate) fn with(self, list: List) -> Self {
        Self(self.0 | Self::bit(list))
    }

    pub(crate) fn contains(self, list: List) -> bool {
        self.0 & Self::bit(list) != 0
    }

    pub(crate) fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn and(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub(crate) fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// The first of the set's lists, in [`ALL_LISTS`]'s order.
    pub(crate) fn first(self) -> Option<List> {
        ALL_LISTS.into_iter().find(|list| self.contains(*list))
    }

    /// The set's lists, in [`ALL_LISTS`]'s order.
    pub(crate) fn lists(self) -> impl Iterator<Item = List> {
        ALL_LISTS
            .into_iter()
            .filter(move |list| self.contains(*list))
    }
}
