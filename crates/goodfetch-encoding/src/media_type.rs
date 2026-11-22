//! A `Content-Type` value: its essence and charset, and which declaration
//! in a document's own bytes its type reads.

/// Which declaration in its own bytes a document's type reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `text/html`: the `<meta>` prescan, then an XML declaration.
    Html,
    /// An XML type: its XML declaration.
    Xml,
    /// Any other type: none.
    Other,
}

impl Kind {
    /// The kind of the MIME type whose essence is `essence`, lowercase.
    #[must_use]
    pub fn of(essence: &str) -> Self {
        match essence {
            "text/html" => Self::Html,
            "application/xhtml+xml" | "text/xml" | "application/xml" => Self::Xml,
            other if other.ends_with("+xml") => Self::Xml,
            _ => Self::Other,
        }
    }
}

/// The essence of the `Content-Type` value `value`, lowercase, and its
/// `charset` parameter, unquoted.
#[must_use]
pub fn content_type(value: &str) -> (String, Option<&str>) {
    let mut parts = value.split(';');
    let essence = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let charset = parts.find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"'))
    });
    (essence, charset)
}

#[cfg(test)]
mod tests;
