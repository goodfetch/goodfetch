//! A crate whose tests drive the protocol and declare what they reach.

#[cfg(test)]
mod tests {
    /// Asserts the module's own Error code, never reaching `rpc`.
    /// Covers: error:sample_refused
    #[test]
    fn refuses_an_empty_text() {}

    /// Sends the request over the in-memory transport.
    /// Covers: operation:sample.echo
    #[test]
    fn echoes_in_process() {}
}
