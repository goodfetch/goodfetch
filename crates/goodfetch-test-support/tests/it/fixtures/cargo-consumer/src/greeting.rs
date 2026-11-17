//! The fixture's greeting.

/// `hello, <name>`.
pub fn greet(name: &str) -> String {
    format!("hello, {name}")
}
