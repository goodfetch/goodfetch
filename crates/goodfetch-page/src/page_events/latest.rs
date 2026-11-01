//! The latest values kept by key, at most `LIMIT`: one past it lets the
//! oldest go (STYLE.md: every buffer is bounded).

use std::collections::{BTreeMap, VecDeque};

/// The latest `LIMIT` values, by key; keeping one again replaces it.
pub(super) struct Latest<V, const LIMIT: usize> {
    values: BTreeMap<String, V>,
    /// The keys, oldest first.
    order: VecDeque<String>,
}

impl<V, const LIMIT: usize> Default for Latest<V, LIMIT> {
    fn default() -> Self {
        Self {
            values: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<V, const LIMIT: usize> Latest<V, LIMIT> {
    /// Keeps `value` under `key`; past the limit, the oldest goes.
    pub(super) fn keep(&mut self, key: String, value: V) {
        if self.values.insert(key.clone(), value).is_some() {
            return;
        }
        self.order.push_back(key);
        while self.order.len() > LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.values.remove(&oldest);
            }
        }
    }

    /// The value under `key`, while it is kept.
    pub(super) fn get(&self, key: &str) -> Option<&V> {
        self.values.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::Latest;

    /// Past its limit, the oldest value goes and the latest stay; keeping
    /// one again replaces it in place.
    #[test]
    fn past_its_limit_the_oldest_goes() {
        let mut latest = Latest::<u32, 2>::default();
        latest.keep("a".to_owned(), 1);
        latest.keep("b".to_owned(), 2);
        latest.keep("b".to_owned(), 3);
        latest.keep("c".to_owned(), 4);

        assert_eq!(
            (latest.get("a"), latest.get("b"), latest.get("c")),
            (None, Some(&3), Some(&4))
        );
    }
}
