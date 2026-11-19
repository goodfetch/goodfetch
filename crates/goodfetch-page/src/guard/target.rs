//! The target renderer (ADR 0079): a locator as the code that builds it in
//! the Client's form (ADR 0099), so an error shows the caller the
//! locator they wrote, and each match of an ambiguous one the locator that
//! reaches it alone. TypeScript keeps Playwright's single quotes; Python,
//! Rust and every form with no language of its own (the CLI's, the
//! tools', the catalog's) take double quotes.

use goodfetch_protocol::connection::Form;

/// `page.locator(selector)`, followed by `.nth(index)` when `nth` is given,
/// as code of `form`.
pub(crate) fn render(form: Form, selector: &str, nth: Option<usize>) -> String {
    let quoted = match form {
        Form::Typescript => single_quoted(selector),
        Form::Rust => format!("{selector:?}"),
        Form::Python | Form::Cli | Form::Tool | Form::Catalog => {
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
