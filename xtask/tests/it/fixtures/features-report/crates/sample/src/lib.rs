//! A crate whose tests declare Features with every outcome a Release run
//! gives them.

#[cfg(test)]
mod tests {
    /// Covers: operation:sample.tested, story:1-2
    #[test]
    fn passes() {}

    /// Covers: operation:sample.failed
    #[test]
    fn fails() {}

    /// Covers: operation:sample.quarantined
    #[test]
    fn flaky() {}

    /// Covers: operation:sample.unrun
    #[test]
    fn never_ran() {}

    /// Covers: operation:sample.crawl
    #[test]
    fn crawls() {}
}
