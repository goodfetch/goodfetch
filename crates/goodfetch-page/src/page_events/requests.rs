//! The requests a page's events follow, by the browser's ID: each one while
//! it is in flight, and once it ended, as one of the latest `LIMIT` that
//! ended (STYLE.md: every buffer is bounded). An older one goes, unless it
//! is the one its owner pins, such as the request of the document the main
//! frame shows, so a page that keeps requesting for hours holds a bounded
//! record of them.

use std::collections::{BTreeMap, VecDeque};

/// The requests in flight, and the latest `LIMIT` that ended, by key.
pub(super) struct Requests<V, const LIMIT: usize> {
    values: BTreeMap<String, V>,
    /// The keys of those that ended, oldest first.
    ended: VecDeque<String>,
}

impl<V, const LIMIT: usize> Default for Requests<V, LIMIT> {
    fn default() -> Self {
        Self {
            values: BTreeMap::new(),
            ended: VecDeque::new(),
        }
    }
}

impl<V, const LIMIT: usize> Requests<V, LIMIT> {
    /// Follows `value` under `key`, in flight: a request sent, or its next
    /// hop, which replaces the hop before.
    pub(super) fn insert(&mut self, key: String, value: V) {
        if self.values.contains_key(&key) {
            self.ended.retain(|ended| *ended != key);
        }
        self.values.insert(key, value);
    }

    /// The request under `key`, while it is kept.
    pub(super) fn get(&self, key: &str) -> Option<&V> {
        self.values.get(key)
    }

    /// The request under `key`, while it is kept, to change.
    pub(super) fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.values.get_mut(key)
    }

    /// The request under `key` ended: past `LIMIT` that ended, the oldest
    /// goes, other than `pinned`.
    pub(super) fn end(&mut self, key: &str, pinned: Option<&str>) {
        if !self.values.contains_key(key) || self.ended.iter().any(|ended| ended == key) {
            return;
        }
        self.ended.push_back(key.to_owned());
        if self.ended.len() <= LIMIT {
            return;
        }
        let oldest = self
            .ended
            .iter()
            .position(|ended| Some(ended.as_str()) != pinned);
        if let Some(oldest) = oldest.and_then(|at| self.ended.remove(at)) {
            self.values.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Requests;

    fn followed(keys: &[&str]) -> Requests<u32, 2> {
        let mut requests = Requests::<u32, 2>::default();
        for (value, key) in (0..).zip(keys) {
            requests.insert((*key).to_owned(), value);
        }
        requests
    }

    fn kept(requests: &Requests<u32, 2>, keys: &[&str]) -> Vec<bool> {
        keys.iter().map(|key| requests.get(key).is_some()).collect()
    }

    /// Past its limit of ended requests, the oldest that ended goes, while
    /// those in flight stay, however many there are.
    #[test]
    fn past_its_limit_the_oldest_ended_request_goes_and_those_in_flight_stay() {
        let mut requests = followed(&["a", "b", "c", "d", "e"]);

        requests.end("a", None);
        requests.end("b", None);
        requests.end("c", None);

        assert_eq!(
            kept(&requests, &["a", "b", "c", "d", "e"]),
            [false, true, true, true, true]
        );
    }

    /// The pinned request stays however many end after it, and the oldest
    /// other one goes in its place.
    #[test]
    fn the_pinned_request_stays_past_the_limit() {
        let mut requests = followed(&["document", "b", "c", "d"]);

        requests.end("document", Some("document"));
        requests.end("b", Some("document"));
        requests.end("c", Some("document"));
        requests.end("d", Some("document"));

        assert_eq!(
            kept(&requests, &["document", "b", "c", "d"]),
            [true, false, false, true]
        );
    }
}
