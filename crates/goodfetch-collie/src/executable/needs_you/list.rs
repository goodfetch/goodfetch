//! What waits for the person, as a session shows it: the engine's "Needs
//! you" items, each keeping the number it was first shown under, changed by
//! the engine's events as another person, or a Run, resolves one.

use std::collections::BTreeSet;

use goodfetch_protocol::needs_you::{NeedsYouChange, NeedsYouItem, NeedsYouRemoval};

use super::kinds::Kinds;

/// One item, under its number.
pub(super) struct Row {
    /// Its number, from 1, never given to another.
    pub(super) number: usize,
    /// It.
    pub(super) item: NeedsYouItem,
    /// Whether the person answered it, since it last changed.
    pub(super) answered: bool,
}

impl Row {
    /// Its lines: its summary, its answers, and what can be done about it
    /// elsewhere.
    pub(super) fn lines(&self, kinds: Kinds) -> Vec<String> {
        let answered = if self.answered { " (answered)" } else { "" };
        let mut lines = vec![format!("{}{answered}", self.heading())];
        let answers = kinds.answers(&self.item);
        if !answers.is_empty() {
            let labels: Vec<&str> = answers.iter().map(|answer| answer.label).collect();
            lines.push(format!("   Answers: {}.", labels.join("; ")));
        }
        lines.extend(
            kinds
                .elsewhere(&self.item)
                .into_iter()
                .map(|said| format!("   {said}")),
        );
        lines
    }

    /// Its number and its summary.
    fn heading(&self) -> String {
        format!("{}. {}", self.number, self.item.summary)
    }
}

/// The items a session holds.
#[derive(Default)]
pub(super) struct List {
    rows: Vec<Row>,
    /// How many numbers it gave.
    given: usize,
}

impl List {
    /// Its rows, in the order of their numbers.
    pub(super) fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Whether nothing waits.
    pub(super) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The item under the number `text` says.
    pub(super) fn numbered(&self, text: &str) -> Option<&NeedsYouItem> {
        let number: usize = text.parse().ok()?;
        self.rows
            .iter()
            .find(|row| row.number == number)
            .map(|row| &row.item)
    }

    /// The item whose ID is `id`.
    pub(super) fn identified(&self, id: &str) -> Option<&NeedsYouItem> {
        self.rows
            .iter()
            .find(|row| row.item.id == id)
            .map(|row| &row.item)
    }

    /// Whether it still holds `item`, unchanged.
    pub(super) fn holds(&self, item: &NeedsYouItem) -> bool {
        self.identified(&item.id) == Some(item)
    }

    /// Notes that the person answered the item `id`.
    pub(super) fn answered(&mut self, id: &str) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.item.id == id) {
            row.answered = true;
        }
    }

    /// Takes one change of the engine's list, and says it.
    pub(super) fn change(&mut self, change: NeedsYouChange, kinds: Kinds) -> Vec<String> {
        match change {
            NeedsYouChange::Listed { item } => self.listed(item, kinds),
            NeedsYouChange::Removed { id, reason } => {
                let reason = match reason {
                    NeedsYouRemoval::Resolved => "resolved",
                    NeedsYouRemoval::Dismissed => "dismissed",
                };
                self.removed(&id, Some(reason))
            }
        }
    }

    /// Takes the engine's whole list, and says what differs.
    pub(super) fn resync(&mut self, items: Vec<NeedsYouItem>, kinds: Kinds) -> Vec<String> {
        let kept: BTreeSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
        let gone: Vec<String> = self
            .rows
            .iter()
            .filter(|row| !kept.contains(row.item.id.as_str()))
            .map(|row| row.item.id.clone())
            .collect();
        let mut news = Vec::new();
        for id in gone {
            news.extend(self.removed(&id, None));
        }
        for item in items {
            news.extend(self.listed(item, kinds));
        }
        news
    }

    fn listed(&mut self, item: NeedsYouItem, kinds: Kinds) -> Vec<String> {
        let (said, row) = match self.rows.iter().position(|row| row.item.id == item.id) {
            Some(at) if self.rows[at].item == item => return Vec::new(),
            Some(at) => {
                let row = &mut self.rows[at];
                row.item = item;
                row.answered = false;
                ("Changed: ", &self.rows[at])
            }
            None => {
                self.given += 1;
                let at = self.rows.len();
                self.rows.push(Row {
                    number: self.given,
                    item,
                    answered: false,
                });
                ("Now on the list: ", &self.rows[at])
            }
        };
        let mut lines = row.lines(kinds);
        if let Some(first) = lines.first_mut() {
            first.insert_str(0, said);
        }
        lines
    }

    fn removed(&mut self, id: &str, reason: Option<&str>) -> Vec<String> {
        let Some(at) = self.rows.iter().position(|row| row.item.id == id) else {
            return Vec::new();
        };
        let row = self.rows.remove(at);
        let reason = reason
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        vec![format!("Left the list: {}{reason}", row.heading())]
    }
}

#[cfg(test)]
mod tests;
