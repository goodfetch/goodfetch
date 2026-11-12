//! An issue body's `Existing code` list, in ADR 0144's form: a heading named
//! exactly `Existing code`, at any level, whose first line is "Read at
//! `<commit>`." and whose items each end with citations in backticks, such as
//! `crates/goodfetch-page/src/page_events.rs:493#arm_runtime`.

/// The list under an issue's `Existing code` heading.
pub(super) struct List {
    /// The commit of "Read at `<commit>`.", when the list's first line is one.
    pub(super) read_at: Option<String>,
    pub(super) items: Vec<Item>,
}

/// One statement of the list.
pub(super) struct Item {
    /// The statement as written, its lines joined.
    pub(super) statement: String,
    /// What it holds in backticks, citations or not.
    pub(super) quoted: Vec<String>,
}

impl Item {
    /// Whether it is the one item of an issue that touches no existing code,
    /// "None: nothing this spec changes exists yet", which cites nothing by
    /// design.
    pub(super) fn says_nothing_exists(&self) -> bool {
        self.statement.starts_with("None:")
    }
}

/// The list of `body`, or `None` when it has no `Existing code` heading. The
/// list runs to the next heading of its level or above.
pub(super) fn find(body: &str) -> Option<List> {
    let mut lines = body.lines();
    let level = lines.by_ref().find_map(existing_code_heading)?;
    let mut read_at = None;
    let mut first = true;
    let mut items: Vec<Item> = Vec::new();
    for line in lines {
        match heading_level(line) {
            Some(other) if other <= level => break,
            // A deeper heading groups items and states nothing.
            Some(_) => continue,
            None => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if std::mem::take(&mut first) {
            read_at = read_commit(trimmed);
            if read_at.is_some() {
                continue;
            }
        }
        let indent = line.len() - line.trim_start().len();
        let bullet = ["- ", "* ", "+ "]
            .iter()
            .find_map(|bullet| trimmed.strip_prefix(bullet));
        match (bullet, items.last_mut()) {
            (Some(statement), _) if indent < 2 => items.push(Item {
                statement: statement.trim().to_owned(),
                quoted: Vec::new(),
            }),
            (_, Some(item)) => {
                item.statement.push(' ');
                item.statement.push_str(trimmed);
            }
            (_, None) => {}
        }
    }
    for item in &mut items {
        item.quoted = quoted(&item.statement);
    }
    Some(List { read_at, items })
}

/// The level of a Markdown heading named exactly `Existing code`.
fn existing_code_heading(line: &str) -> Option<usize> {
    let level = heading_level(line)?;
    let name = line[level..].trim().trim_end_matches('#').trim();
    (name == "Existing code").then_some(level)
}

/// The level of an ATX heading: one to six `#` and a space.
fn heading_level(line: &str) -> Option<usize> {
    let level = line.bytes().take_while(|&byte| byte == b'#').count();
    let rest = &line[level..];
    ((1..=6).contains(&level) && (rest.is_empty() || rest.starts_with(' '))).then_some(level)
}

/// The commit of a "Read at `<commit>`." line, at least 7 hexadecimal
/// digits, whatever follows it on the line.
fn read_commit(line: &str) -> Option<String> {
    let (commit, _) = line.strip_prefix("Read at `")?.split_once('`')?;
    ((7..=40).contains(&commit.len()) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| commit.to_ascii_lowercase())
}

/// What `text` holds between backticks.
fn quoted(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| !span.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A citation's parts: a path from the repository's root, an optional line or
/// range of lines and an optional symbol.
pub(super) struct Citation {
    /// As written, for the output.
    pub(super) written: String,
    pub(super) path: String,
    /// The first and last line, from 1.
    pub(super) lines: Option<(usize, usize)>,
    pub(super) symbol: Option<String>,
}

impl Citation {
    /// `quoted` read as a citation's form, `path[:N[-M]][#symbol]`; `None`
    /// when it has another form. Whether its path is the repository's is the
    /// caller's to ask.
    pub(super) fn parse(quoted: &str) -> Option<Self> {
        let (located, symbol) = match quoted.split_once('#') {
            Some((located, symbol)) if !symbol.trim().is_empty() => {
                (located, Some(symbol.trim().to_owned()))
            }
            Some(_) => return None,
            None => (quoted, None),
        };
        let (path, lines) = match located.split_once(':') {
            Some((path, lines)) => (path, Some(range(lines)?)),
            // A folder may end with `/`, as in `journeys/`.
            None => (located.strip_suffix('/').unwrap_or(located), None),
        };
        let path_character =
            |character: char| character.is_ascii_alphanumeric() || "_.-/+@".contains(character);
        if path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|part| part.is_empty() || part == "..")
            || !path.chars().all(path_character)
        {
            return None;
        }
        Some(Self {
            written: quoted.to_owned(),
            path: path.to_owned(),
            lines,
            symbol,
        })
    }

    /// The path's first part: a root file or a top folder.
    pub(super) fn top(&self) -> &str {
        self.path.split('/').next().unwrap_or(&self.path)
    }
}

/// `N` or `N-M`, from line 1, `M` not before `N`.
fn range(text: &str) -> Option<(usize, usize)> {
    let (first, last) = text.split_once('-').unwrap_or((text, text));
    let first: usize = first.parse().ok()?;
    let last: usize = last.parse().ok()?;
    (first >= 1 && last >= first).then_some((first, last))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{Citation, find};

    #[test]
    fn a_list_ends_at_the_next_heading_of_its_level() {
        let body = "# Spec\n\n### Existing code\n\nRead at `95935D9B2`, with #2523's reading.\n\n- One. `a/b.rs:3`\n  goes on.\n  - nested `c.rs`\n* Two. `d`\n#### Deeper\n- Three.\n## Next\n- Not an item.\n";

        let list = find(body).unwrap();

        assert_eq!(list.read_at.as_deref(), Some("95935d9b2"));
        let statements: Vec<_> = list.items.iter().map(|item| &item.statement).collect();
        assert_eq!(
            statements,
            [
                "One. `a/b.rs:3` goes on. - nested `c.rs`",
                "Two. `d`",
                "Three."
            ]
        );
        assert_eq!(list.items[0].quoted, ["a/b.rs:3", "c.rs"]);
    }

    #[rstest]
    #[case::too_short_a_commit("Read at `95935d`.")]
    #[case::a_commit_out_of_backticks("Read at 95935d9b2.")]
    #[case::an_item("- One. `a.rs`")]
    fn a_first_line_that_is_no_read_commit_leaves_none(#[case] first: &str) {
        let list = find(&format!("## Existing code\n\n{first}\n")).unwrap();

        assert_eq!(list.read_at, None);
    }

    #[test]
    fn a_heading_that_only_starts_with_existing_code_holds_no_list() {
        assert!(find("## Existing code paths\n").is_none());
    }

    #[test]
    fn a_citation_has_a_path_lines_and_a_symbol() {
        let citation = Citation::parse("xtask/src/issues.rs:24-29#fn keep").unwrap();

        assert_eq!(citation.path, "xtask/src/issues.rs");
        assert_eq!(citation.lines, Some((24, 29)));
        assert_eq!(citation.symbol.as_deref(), Some("fn keep"));
        assert_eq!(citation.top(), "xtask");
        assert_eq!(Citation::parse("ci/musl/cargo").unwrap().lines, None);
    }

    #[rstest]
    #[case::a_call("page.on()")]
    #[case::an_address("https://example.com")]
    #[case::line_zero("a:0")]
    #[case::a_backward_range("a:5-4")]
    #[case::an_issue("#2242")]
    #[case::an_absolute_path("/etc/hosts")]
    #[case::a_parent_folder("../a")]
    #[case::a_space("a b")]
    fn other_forms_are_no_citation(#[case] quoted: &str) {
        assert!(Citation::parse(quoted).is_none());
    }
}
