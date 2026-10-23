//! The shedding ladder's rules (ADR 0040), pure: which browsers the
//! governor sheds at critical, and which Run its last resort stops. At
//! each critical sample every idle browser closes, then the largest that
//! has open pages is recycled, one a sample so the next is measured again;
//! a held browser is never shed, nor one shed already. The last resort
//! stops the Run with the largest footprint, the first of equals, never
//! one holding a page under a person's control: that Run is stopped last,
//! once the person hands back or leaves.

/// What the ladder weighs of one browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// Its number.
    pub(crate) id: u64,
    pub(crate) open_pages: usize,
    /// Whether it holds a held page.
    pub(crate) held: bool,
    /// What it used at the last sample.
    pub(crate) footprint: u64,
    /// Whether it was shed already.
    pub(crate) shed: bool,
}

/// The numbers of the browsers, among `browsers` in order, to shed at a
/// critical sample: every idle one, then the largest with open pages.
pub(crate) fn browsers_to_shed(browsers: &[Candidate]) -> Vec<u64> {
    let sheddable = || {
        browsers
            .iter()
            .filter(|browser| !browser.held && !browser.shed)
    };
    let mut shed: Vec<u64> = sheddable()
        .filter(|browser| browser.open_pages == 0)
        .map(|browser| browser.id)
        .collect();
    let largest = sheddable().filter(|browser| browser.open_pages > 0).fold(
        None,
        |largest: Option<&Candidate>, browser| match largest {
            Some(largest) if largest.footprint >= browser.footprint => Some(largest),
            Some(_) | None => Some(browser),
        },
    );
    shed.extend(largest.map(|browser| browser.id));
    shed
}

/// What the last resort weighs of a live Run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weight {
    /// Its browsers' memory at the governor's last sample, in bytes.
    pub footprint: u64,
    /// Whether a person controls one of its pages in the Live view.
    pub held_by_person: bool,
}

/// The Run, among `runs` in order, that the last resort stops: the one with
/// the largest footprint, the first of equals, never one a person holds;
/// none when only such Runs are left.
pub fn last_resort_run<K>(runs: impl IntoIterator<Item = (K, Weight)>) -> Option<K> {
    runs.into_iter()
        .filter(|(_, weight)| !weight.held_by_person)
        .fold(
            None,
            |largest: Option<(K, Weight)>, (run, weight)| match largest {
                Some(largest) if largest.1.footprint >= weight.footprint => Some(largest),
                Some(_) | None => Some((run, weight)),
            },
        )
        .map(|(run, _)| run)
}
