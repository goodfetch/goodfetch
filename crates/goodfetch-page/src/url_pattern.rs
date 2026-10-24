//! The engine's one URL matcher, for waits, routes and kept responses
//! (ADRs 0081 and 0110): a glob read as Playwright 1.64.1 reads one, or a
//! regular expression in the `regex` crate's syntax, so a pattern matches
//! the same URLs whichever SDK passed it. A construct outside that syntax
//! (look-around, a back-reference) is refused when the pattern is compiled,
//! so the call that passed it raises `pattern_unsupported`; a predicate
//! runs in the program instead. Mapping a TypeScript `RegExp`'s or a Python
//! `re.Pattern`'s flags and classes into this syntax is the SDKs' (area 11).

use goodfetch_engine::CallError;
use goodfetch_protocol::page::{PATTERN_UNSUPPORTED, PatternConstruct, PatternUnsupportedDetails};
use regex::Regex;
use url::Url;

/// A compiled URL pattern.
#[derive(Clone, Debug)]
pub struct UrlPattern {
    regex: Regex,
}

impl UrlPattern {
    /// A glob, read as Playwright reads a `string` URL pattern with no base
    /// URL: unless it starts with `*`, resolved as a URL first, so
    /// `http://site.test` matches `http://site.test/` and a host matches in
    /// any case; then `*` matches within one path segment, `**` across
    /// segments, `{a,b}` either, and `\` escapes the next character. `?`
    /// and `[` match themselves. An empty glob matches every URL.
    ///
    /// # Errors
    ///
    /// [`UnsupportedPattern`] for nested or unmatched braces, which
    /// Playwright refuses too.
    pub fn glob(glob: &str) -> Result<Self, UnsupportedPattern> {
        if glob.is_empty() {
            return Ok(Self {
                regex: Regex::new("").expect("invariant: the empty expression compiles"),
            });
        }
        let source = glob_source(&resolve(glob)).map_err(|construct| UnsupportedPattern {
            pattern: glob.to_owned(),
            construct,
            description: match construct {
                PatternConstruct::NestedBraces => "a `{` inside a `{…}` group".to_owned(),
                _ => "a `{` or `}` with no match".to_owned(),
            },
        })?;
        let regex = Regex::new(&source).map_err(|error| UnsupportedPattern {
            pattern: glob.to_owned(),
            construct: PatternConstruct::Other,
            description: format!("a glob too large to compile ({error})"),
        })?;
        Ok(Self { regex })
    }

    /// A regular expression in the `regex` crate's syntax, which matches a
    /// URL when it matches anywhere in it, as JavaScript's `test` does.
    ///
    /// # Errors
    ///
    /// [`UnsupportedPattern`] naming the construct, for look-around, a
    /// back-reference or any other construct outside that syntax.
    pub fn regex(source: &str) -> Result<Self, UnsupportedPattern> {
        let unsupported = |construct, description| UnsupportedPattern {
            pattern: source.to_owned(),
            construct,
            description,
        };
        if let Err(error) = regex_syntax::Parser::new().parse(source) {
            let (construct, description) = construct_of(&error);
            return Err(unsupported(construct, description));
        }
        let regex = Regex::new(source).map_err(|error| {
            unsupported(
                PatternConstruct::Other,
                format!("a regular expression too large to compile ({error})"),
            )
        })?;
        Ok(Self { regex })
    }

    /// Whether `url` matches.
    pub fn matches(&self, url: &str) -> bool {
        self.regex.is_match(url)
    }
}

/// A glob's `**`: Playwright's `.*`, any characters but a line
/// terminator, as JavaScript's `.` reads them.
const ANYTHING: &str = r"[^\n\r\x{2028}\x{2029}]*";

/// A glob's `**/` after anything but a `/`: Playwright's `.*/`.
const ANYTHING_THEN_SLASH: &str = r"[^\n\r\x{2028}\x{2029}]*/";

/// A glob's `/**/`'s `**/`: Playwright's `((.+/)|)`, so that `/**/` also
/// matches a single `/`.
const NO_SEGMENT_OR_SEGMENTS: &str = r"(?:[^\n\r\x{2028}\x{2029}]+/|)";

/// The schemes of the globs Playwright matches as written, never resolved.
const UNRESOLVED_SCHEMES: [&str; 5] = ["about:", "data:", "chrome:", "edge:", "file:"];

/// `glob` resolved as a URL, as Playwright's `resolveGlobBase` resolves it
/// with no base URL: each path segment holding a glob character is set
/// aside behind a placeholder, the rest parsed as a WHATWG URL (so an
/// origin gains its `/`, a default port goes and a host is lowercased),
/// and the segments put back, lowercased when they sit in the origin. A
/// glob that is no absolute URL stays as written.
fn resolve(glob: &str) -> String {
    if glob.starts_with('*') {
        return glob.to_owned();
    }
    let glob = glob.replace(r"\\?", "?");
    if UNRESOLVED_SCHEMES
        .iter()
        .any(|scheme| glob.starts_with(scheme))
    {
        return glob;
    }
    let mut placeholders = Placeholders::default();
    let relative: Vec<String> = glob
        .split('/')
        .enumerate()
        .map(|(index, segment)| {
            if matches!(segment, "." | ".." | "") {
                return segment.to_owned();
            }
            if index == 0 && segment.ends_with(':') {
                if segment.contains(['*', '{']) {
                    return placeholders.set_aside(segment, "http:".to_owned());
                }
                return segment.to_owned();
            }
            if !segment.contains(['*', '?', '{', '}', '\\']) {
                return segment.to_owned();
            }
            match segment.find('?') {
                None => placeholders.set_aside(segment, format!("$_{index}_$")),
                Some(query) => {
                    placeholders.set_aside(&segment[..query], format!("$_{index}_$"))
                        + &placeholders.set_aside(&segment[query..], format!("?$_{index}_$"))
                }
            }
        })
        .collect();
    let relative = relative.join("/");
    let (mut resolved, origin) = match Url::parse(&relative) {
        Ok(url) => (
            url.as_str().to_owned(),
            Some(url.origin().ascii_serialization()),
        ),
        Err(_) => (relative, None),
    };
    for (placeholder, original) in placeholders.0 {
        let original = if origin
            .as_deref()
            .is_some_and(|origin| origin.contains(&placeholder))
        {
            original.to_lowercase()
        } else {
            original
        };
        resolved = resolved.replacen(&placeholder, &original, 1);
    }
    resolved
}

/// The glob segments [`resolve`] set aside, by placeholder, in the order
/// they were first set aside.
#[derive(Default)]
struct Placeholders(Vec<(String, String)>);

impl Placeholders {
    /// Sets `original` aside behind `placeholder`, which it returns; an
    /// empty segment needs none.
    fn set_aside(&mut self, original: &str, placeholder: String) -> String {
        if original.is_empty() {
            return String::new();
        }
        match self.0.iter_mut().find(|(kept, _)| *kept == placeholder) {
            Some(entry) => original.clone_into(&mut entry.1),
            None => self.0.push((placeholder.clone(), original.to_owned())),
        }
        placeholder
    }
}

/// The regular expression of a resolved glob, as Playwright's
/// `globToRegexPattern` writes it: `*` within one path segment, `**`
/// across them (`/**/` matching a single `/` too), `{a,b}` either, `\`
/// escaping the next character, and every other character itself.
///
/// # Errors
///
/// The construct, for nested or unmatched braces.
fn glob_source(glob: &str) -> Result<String, PatternConstruct> {
    let characters: Vec<char> = glob.chars().collect();
    let mut source = String::from("^");
    let mut in_group = false;
    let mut index = 0;
    while let Some(&character) = characters.get(index) {
        let before = index
            .checked_sub(1)
            .and_then(|before| characters.get(before));
        index += 1;
        match character {
            '\\' if index < characters.len() => {
                source.push_str(&regex::escape(&characters[index].to_string()));
                index += 1;
            }
            '*' => {
                let mut stars = 1;
                while characters.get(index) == Some(&'*') {
                    stars += 1;
                    index += 1;
                }
                if stars == 1 {
                    source.push_str("[^/]*");
                } else if characters.get(index) == Some(&'/') {
                    index += 1;
                    if before == Some(&'/') {
                        source.push_str(NO_SEGMENT_OR_SEGMENTS);
                    } else {
                        source.push_str(ANYTHING_THEN_SLASH);
                    }
                } else {
                    source.push_str(ANYTHING);
                }
            }
            '{' if in_group => return Err(PatternConstruct::NestedBraces),
            '{' => {
                in_group = true;
                source.push_str("(?:");
            }
            '}' if !in_group => return Err(PatternConstruct::UnmatchedBrace),
            '}' => {
                in_group = false;
                source.push(')');
            }
            ',' if in_group => source.push('|'),
            _ => source.push_str(&regex::escape(&character.to_string())),
        }
    }
    if in_group {
        return Err(PatternConstruct::UnmatchedBrace);
    }
    source.push('$');
    Ok(source)
}

/// How a back-reference by name opens: JavaScript's and Python's.
const NAMED_BACK_REFERENCES: [&str; 2] = ["\\k<", "(?P="];

/// The text of the pattern from where the parser's `error` points on,
/// or, for a group, from the group's opening.
fn refused_text(error: &regex_syntax::ast::Error) -> &str {
    let pattern = error.pattern();
    let start = error.span().start.offset;
    // The parser points inside `(?P=name)`, past its opening.
    let within = (start + "(?P=".len()).min(pattern.len());
    let group = pattern
        .get(..within)
        .and_then(|before| before.rfind("(?P="))
        .filter(|opened| !pattern[*opened..start.max(*opened)].contains(')'));
    pattern.get(group.unwrap_or(start)..).unwrap_or_default()
}

/// The construct of a regular expression the `regex` crate's parser
/// refused, and how the message names it.
fn construct_of(error: &regex_syntax::Error) -> (PatternConstruct, String) {
    match error {
        regex_syntax::Error::Parse(error) => match error.kind() {
            regex_syntax::ast::ErrorKind::UnsupportedLookAround => (
                PatternConstruct::LookAround,
                "look-around (a look-ahead or look-behind)".to_owned(),
            ),
            // JavaScript's `\0` is the null character, never a group.
            regex_syntax::ast::ErrorKind::UnsupportedBackreference
                if refused_text(error).starts_with("\\0") =>
            {
                (
                    PatternConstruct::Other,
                    "`\\0`, JavaScript's null character, which the `regex` crate writes `\\x00`"
                        .to_owned(),
                )
            }
            regex_syntax::ast::ErrorKind::UnsupportedBackreference => (
                PatternConstruct::Backreference,
                "a back-reference".to_owned(),
            ),
            // A back-reference by name, JavaScript's `\k<name>` or Python's
            // `(?P=name)`, which the parser reads as another construct.
            _ if NAMED_BACK_REFERENCES
                .iter()
                .any(|opening| refused_text(error).starts_with(opening)) =>
            {
                (
                    PatternConstruct::Backreference,
                    "a back-reference by name".to_owned(),
                )
            }
            kind => (
                PatternConstruct::Other,
                format!("a construct outside the `regex` crate's syntax ({kind})"),
            ),
        },
        regex_syntax::Error::Translate(error) => (
            PatternConstruct::Other,
            format!(
                "a construct outside the `regex` crate's syntax ({})",
                error.kind()
            ),
        ),
        error => (
            PatternConstruct::Other,
            format!("a construct outside the `regex` crate's syntax ({error})"),
        ),
    }
}

/// A URL pattern the matcher cannot read, which the call that passed it
/// raises as `pattern_unsupported`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the URL pattern {pattern:?} uses {description}, which the engine cannot match")]
#[non_exhaustive]
pub struct UnsupportedPattern {
    pattern: String,
    construct: PatternConstruct,
    description: String,
}

impl UnsupportedPattern {
    /// The construct it uses.
    pub fn construct(&self) -> PatternConstruct {
        self.construct
    }
}

impl From<UnsupportedPattern> for CallError {
    fn from(unsupported: UnsupportedPattern) -> Self {
        let message = unsupported.to_string();
        Self::new(
            &PATTERN_UNSUPPORTED,
            message,
            &PatternUnsupportedDetails {
                pattern: unsupported.pattern,
                construct: unsupported.construct,
                fix: "match the URL with a predicate, which runs in the program".to_owned(),
            },
        )
    }
}

#[cfg(test)]
mod tests;
