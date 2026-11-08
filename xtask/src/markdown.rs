//! Sections and tables of the Markdown documents xtask reads its rules
//! from (`TESTING.md`'s Quality register and time budgets, ADR 0037's
//! matrix, ADR 0140's inventory). A section is what lies under a heading
//! line up to the next heading of its level or above, outside fenced code;
//! a table is a run of lines that start with `|`, its header first and its
//! separator second; a cell is what lies between two `|`, trimmed.

/// The trimmed cells of a table line: what lies between its first `|` and
/// its last; `None` for a line that does not start with `|`.
pub(crate) fn cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.trim().strip_prefix('|')?;
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    Some(inner.split('|').map(str::trim).collect())
}

/// The level of a heading line (`### Floor` is 3); `None` for another line.
fn level(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    let heading = (1..=6).contains(&hashes) && line[hashes..].starts_with(' ');
    heading.then_some(hashes)
}

/// The lines under the heading line `heading` (`### Floor`, matched whole,
/// trailing spaces aside) among `lines`, up to the next heading of its level
/// or above; `None` when no line outside fenced code is that heading.
pub(crate) fn section<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    heading: &str,
) -> Option<Vec<&'a str>> {
    let wanted = level(heading)?;
    let mut fenced = false;
    let mut found_lines = None;
    for line in lines {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let found = if fenced { None } else { level(line) };
        match (&mut found_lines, found) {
            (None, Some(_)) if line.trim_end() == heading => found_lines = Some(Vec::new()),
            (Some(_), Some(found)) if found <= wanted => break,
            (Some(under), _) => under.push(line),
            (None, _) => {}
        }
    }
    found_lines
}

/// One table: its header's cells, then each row below its separator.
pub(crate) struct Table<'a> {
    pub(crate) header: Vec<&'a str>,
    pub(crate) rows: Vec<Row<'a>>,
}

/// One row of a table: its line, and its cells.
pub(crate) struct Row<'a> {
    pub(crate) line: &'a str,
    pub(crate) cells: Vec<&'a str>,
}

impl Table<'_> {
    /// The position of the column `name` in the header.
    pub(crate) fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|cell| *cell == name)
    }
}

/// The first table of `lines`; `None` when they hold none.
pub(crate) fn first_table<'a>(lines: &[&'a str]) -> Option<Table<'a>> {
    let mut table = lines
        .iter()
        .copied()
        .skip_while(|line| !line.trim_start().starts_with('|'))
        .take_while(|line| line.trim_start().starts_with('|'));
    let header = cells(table.next()?)?;
    let rows = table
        // The header's separator.
        .skip(1)
        .filter_map(|line| {
            Some(Row {
                line,
                cells: cells(line)?,
            })
        })
        .collect();
    Some(Table { header, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "\
# Title

## Register

Prose.

```sh
# a comment, no heading
```

### Floor

- a rule

#### Detail

- a detail

### Table

| Name | Value |
|---|---|
| `a` | 1 |
| b | |

## After
";

    #[test]
    fn a_section_ends_at_the_next_heading_of_its_level_or_above() {
        assert_eq!(
            section(DOCUMENT.lines(), "### Floor"),
            Some(vec![
                "",
                "- a rule",
                "",
                "#### Detail",
                "",
                "- a detail",
                ""
            ])
        );
    }

    #[test]
    fn a_comment_in_fenced_code_is_no_heading() {
        let register = section(DOCUMENT.lines(), "## Register").unwrap();

        assert!(register.contains(&"### Table"));
    }

    #[test]
    fn a_heading_quoted_in_prose_is_no_section() {
        assert_eq!(
            section("A reader names `## Register`.\n".lines(), "## Register"),
            None
        );
    }

    #[test]
    fn a_table_reads_its_header_and_each_row_below_its_separator() {
        let lines = section(DOCUMENT.lines(), "### Table").unwrap();

        let table = first_table(&lines).unwrap();

        assert_eq!(
            (
                table.header,
                table
                    .rows
                    .iter()
                    .map(|row| row.cells.clone())
                    .collect::<Vec<_>>()
            ),
            (vec!["Name", "Value"], vec![vec!["`a`", "1"], vec!["b", ""]])
        );
    }
}
