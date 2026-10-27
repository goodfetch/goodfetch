//! The links to the Console the engine builds (ADR 0031): a run's
//! `console_url` and, once they exist, the links in Notifications. Each
//! starts with the address the person set, `console.public_url`, or else
//! the Console's own, and carries no credential, so a forwarded link grants
//! nothing.

/// The address every link to the Console starts with: the Console's
/// `public_url` when one is set, else its own `url`, with no trailing `/`,
/// so a link is `<base>/runs/<run_id>`. Its query and fragment are dropped,
/// so no token or other credential a URL may hold reaches a link.
pub fn link_base(url: &str, public_url: Option<&str>) -> String {
    let base = public_url
        .filter(|public| !public.is_empty())
        .unwrap_or(url);
    let base = base.split(['?', '#']).next().unwrap_or(base);
    base.trim_end_matches('/').to_owned()
}

#[cfg(test)]
mod tests;
