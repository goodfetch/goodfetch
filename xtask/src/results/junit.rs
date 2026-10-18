//! The test cases of a JUnit XML file, named the way each runner's own
//! listing names them (the test quarantine list's IDs, TESTING.md):
//! - nextest: `<binary ID> <test name>`, its test suite then its test case;
//! - Vitest: the test case's name, its suites and title joined by ` > `;
//! - Playwright: the file (the class name), then the name, joined by ` › `;
//! - pytest: the node ID, `<module path>.py::<name>`, from the dotted class
//!   name;
//! - any other runner: the class name and the name, joined by a space.

use anyhow::{Context as _, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// How a test case ended, as its JUnit file says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ending {
    Passed,
    /// A `<failure>` or an `<error>`.
    Failed,
    Skipped,
}

pub(crate) struct TestCase {
    pub(crate) test: String,
    pub(crate) ending: Ending,
    pub(crate) duration_seconds: f64,
}

/// The runner that wrote a JUnit file, from its name: the part before the
/// first `-` or `.` (`nextest-ci.xml`, `vitest.xml`).
pub(crate) fn runner(file_name: &str) -> &str {
    let stem = file_name.strip_suffix(".xml").unwrap_or(file_name);
    stem.split(['-', '.']).next().unwrap_or(stem)
}

/// Every test case of `text`, in document order.
pub(crate) fn test_cases(runner: &str, text: &str) -> Result<Vec<TestCase>> {
    let mut reader = Reader::from_str(text);
    let mut suite = String::new();
    let mut cases = Vec::new();
    let mut open: Option<TestCase> = None;
    loop {
        let event = reader.read_event().context("parsing the JUnit XML")?;
        match event {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"testsuite" =>
            {
                suite = attribute(&element, "name")?.unwrap_or_default();
            }
            Event::Start(element) if element.local_name().as_ref() == b"testcase" => {
                open = Some(test_case(runner, &suite, &element)?);
            }
            Event::Empty(element) if element.local_name().as_ref() == b"testcase" => {
                cases.push(test_case(runner, &suite, &element)?);
            }
            Event::Start(element) | Event::Empty(element) => {
                if let Some(case) = open.as_mut() {
                    match element.local_name().as_ref() {
                        b"failure" | b"error" => case.ending = Ending::Failed,
                        b"skipped" if case.ending == Ending::Passed => {
                            case.ending = Ending::Skipped;
                        }
                        _ => {}
                    }
                }
            }
            Event::End(element) if element.local_name().as_ref() == b"testcase" => {
                cases.extend(open.take());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(cases)
}

fn test_case(runner: &str, suite: &str, element: &BytesStart<'_>) -> Result<TestCase> {
    let name = attribute(element, "name")?.context("a test case has no name")?;
    let class = attribute(element, "classname")?.unwrap_or_default();
    let test = match runner {
        "nextest" => format!("{suite} {name}"),
        "vitest" => name,
        "playwright" => format!("{class} › {name}"),
        "pytest" => pytest_node_id(&class, &name),
        _ if class.is_empty() => name,
        _ => format!("{class} {name}"),
    };
    let duration_seconds = match attribute(element, "time")? {
        Some(time) => time
            .parse()
            .with_context(|| format!("test case `{test}` has the time `{time}`"))?,
        None => 0.0,
    };
    Ok(TestCase {
        test,
        ending: Ending::Passed,
        duration_seconds,
    })
}

/// pytest's node ID from its JUnit class name: `tests.test_page` gives
/// `tests/test_page.py::<name>`, and a last part in `CamelCase` is a class,
/// so `tests.test_page.TestTitle` gives `tests/test_page.py::TestTitle::<name>`.
fn pytest_node_id(class: &str, name: &str) -> String {
    let mut parts: Vec<&str> = class.split('.').collect();
    let mut classes = Vec::new();
    while parts.len() > 1
        && parts
            .last()
            .is_some_and(|part| part.starts_with(|first: char| first.is_ascii_uppercase()))
    {
        classes.extend(parts.pop());
    }
    classes.reverse();
    let mut id = format!("{}.py", parts.join("/"));
    for class in classes {
        id.push_str("::");
        id.push_str(class);
    }
    format!("{id}::{name}")
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute.context("parsing an attribute")?;
        if attribute.key.local_name().as_ref() == name.as_bytes() {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}
