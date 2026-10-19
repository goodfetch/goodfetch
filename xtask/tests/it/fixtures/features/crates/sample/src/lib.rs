//! A crate whose tests declare Features from each place a test sits.

mod store;

#[cfg(test)]
mod tests {
    /// Covers: operation:sample.echo
    #[test]
    fn echo_repeats_the_text() {}

    #[test]
    fn helper_declares_nothing() {}
}
