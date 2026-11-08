//! The equivalent forms of a URL (ADR 0120; spec #581's thresholds): one
//! leading `www.` label against the bare host, `http` against `https`, a
//! final `index.html` or `index.php` against its folder, and a trailing
//! slash against none. Each is two spellings a site may serve as one page
//! or as two, so the Frontier merges them only once it learned, per host,
//! that they are one: these functions say which other spelling a URL has,
//! by which form two URLs differ, and which spelling a learned form keeps.
//! A `www` form never crosses a registrable domain, so a host whose bare
//! name is a public suffix has none.

use goodfetch_protocol::foxhound::report::EquivalentForm;
use url::Url;

/// Every form, in the order a URL is merged under the learned ones: the
/// host, then the scheme, then the index file, then the trailing slash,
/// so `http://www.site.test/docs/index.html` meets `https://site.test/docs`
/// once all of them are learned.
pub(crate) const FORMS: [EquivalentForm; 5] = [
    EquivalentForm::Www,
    EquivalentForm::Scheme,
    EquivalentForm::IndexHtml,
    EquivalentForm::IndexPhp,
    EquivalentForm::TrailingSlash,
];

const WWW: &str = "www.";

/// The form's name, as the run file and the report write it.
pub(crate) fn name(form: EquivalentForm) -> &'static str {
    match form {
        EquivalentForm::Www => "www",
        EquivalentForm::Scheme => "scheme",
        EquivalentForm::TrailingSlash => "trailing_slash",
        EquivalentForm::IndexHtml => "index_html",
        EquivalentForm::IndexPhp => "index_php",
    }
}

/// The host a rule of `form` learned on `url` holds on: the bare host for
/// `www`, `url`'s host for the others.
pub(crate) fn rule_host(form: EquivalentForm, url: &Url) -> Option<String> {
    let host = url.host_str()?;
    match form {
        EquivalentForm::Www => Some(www_pair(url)?.0),
        _ => Some(host.to_owned()),
    }
}

/// The bare host and the `www.` host of `url`'s host, when both are names
/// of one registrable domain.
fn www_pair(url: &Url) -> Option<(String, String)> {
    let host = url.domain()?;
    let (bare, www) = match host.strip_prefix(WWW) {
        Some(bare) if !bare.is_empty() => (bare.to_owned(), host.to_owned()),
        _ => (host.to_owned(), format!("{WWW}{host}")),
    };
    let registrable = psl::domain(bare.as_bytes())?;
    (psl::domain(www.as_bytes()) == Some(registrable)).then_some((bare, www))
}

/// The file name of an index form.
fn index_file(form: EquivalentForm) -> Option<&'static str> {
    match form {
        EquivalentForm::IndexHtml => Some("index.html"),
        EquivalentForm::IndexPhp => Some("index.php"),
        _ => None,
    }
}

/// `url` in the other spelling of `form`, when it has one; its query
/// stays.
pub(crate) fn other(form: EquivalentForm, url: &Url) -> Option<Url> {
    let mut other = url.clone();
    match form {
        EquivalentForm::Www => {
            let (bare, www) = www_pair(url)?;
            let to = if url.host_str() == Some(bare.as_str()) {
                www
            } else {
                bare
            };
            other.set_host(Some(&to)).ok()?;
        }
        EquivalentForm::Scheme => {
            let to = match url.scheme() {
                "http" => "https",
                "https" => "http",
                _ => return None,
            };
            other.set_scheme(to).ok()?;
        }
        EquivalentForm::TrailingSlash => {
            let path = url.path();
            if path == "/" || path == "//" {
                return None;
            }
            match path.strip_suffix('/') {
                Some(without) => other.set_path(without),
                None => other.set_path(&format!("{path}/")),
            }
        }
        EquivalentForm::IndexHtml | EquivalentForm::IndexPhp => {
            let file = index_file(form)?;
            let path = url.path();
            if let Some(folder) = path
                .strip_suffix(file)
                .filter(|folder| folder.ends_with('/'))
            {
                other.set_path(folder);
            } else if path.ends_with('/') {
                other.set_path(&format!("{path}{file}"));
            } else {
                return None;
            }
        }
    }
    (other != *url).then_some(other)
}

/// The form by which `one` and `other` differ, and nothing else, with the
/// host its rule would hold on.
pub(crate) fn differ_by(one: &Url, other: &Url) -> Option<(EquivalentForm, String)> {
    FORMS.into_iter().find_map(|form| {
        (self::other(form, one).as_ref() == Some(other))
            .then(|| rule_host(form, one))
            .flatten()
            .map(|host| (form, host))
    })
}

/// `url` in the spelling a learned `form` keeps, when it is in the other
/// one: the bare host, `https`, the folder and no trailing slash.
pub(crate) fn kept(form: EquivalentForm, url: &Url) -> Option<Url> {
    let path = url.path();
    let merged = match form {
        EquivalentForm::Www => url
            .host_str()
            .is_some_and(|host| host.starts_with(WWW) && www_pair(url).is_some()),
        EquivalentForm::Scheme => url.scheme() == "http",
        EquivalentForm::TrailingSlash => path != "/" && path != "//" && path.ends_with('/'),
        EquivalentForm::IndexHtml | EquivalentForm::IndexPhp => {
            index_file(form).is_some_and(|file| {
                path.strip_suffix(file)
                    .is_some_and(|folder| folder.ends_with('/'))
            })
        }
    };
    merged.then(|| other(form, url)).flatten()
}

#[cfg(test)]
mod tests;
