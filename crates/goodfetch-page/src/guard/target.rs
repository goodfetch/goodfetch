//! The target renderer (ADR 0079): a locator as the code that builds it in
//! the language of the Client's surface, so an error shows the caller the
//! locator they wrote, and each match of an ambiguous one the locator that
//! reaches it alone. TypeScript keeps Playwright's single quotes; Python,
//! Rust and every surface with no language of its own (the CLI, MCP) take
//! the catalog's form, with double quotes.

use goodfetch_protocol::connection::Surface;

/// `page.locator(selector)`, followed by `.nth(index)` when `nth` is given,
/// as code of `surface`'s language.
pub(crate) fn render(surface: Surface, selector: &str, nth: Option<usize>) -> String {
    let quoted = match surface {
        Surface::SdkTypescript => single_quoted(selector),
        Surface::Rust => format!("{selector:?}"),
        Surface::SdkPython
        | Surface::Cli
        | Surface::Mcp
        | Surface::Kennel
        | Surface::Cloud
        | Surface::BrowserHost
        | Surface::Worker => {
            serde_json::to_string(selector).expect("invariant: a string encodes as JSON")
        }
    };
    match nth {
        Some(index) => format!("page.locator({quoted}).nth({index})"),
        None => format!("page.locator({quoted})"),
    }
}

/// A JavaScript string literal in single quotes.
fn single_quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            other => quoted.push(other),
        }
    }
    quoted.push('\'');
    quoted
}
