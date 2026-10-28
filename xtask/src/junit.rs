//! The test cases of a JUnit XML file, named the way each runner's own
//! listing names them (the test quarantine list's IDs, TESTING.md):
//! - nextest: `<binary ID> <test name>`, its test suite then its test case;
//! - Vitest: the test case's name, its suites and title joined by ` > `;
//! - Playwright: the file (the class name), then the name, joined by ` › `;
//! - pytest: the node ID, `<module path>.py::<name>`, from the dotted class
//!   name;
//! - any other runner: the class name and the name, joined by a space.
//!
//! Each case keeps its first failure and what it printed, which the failure
//! summary shows. A file that is not well-formed XML, or holds no test
//! suite, is an error, never an empty file.

use anyhow::{Context as _, Result, bail};
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

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
    /// Its `<failure>` and `<error>` elements: one per failed run of a test
    /// Vitest repeated.
    pub(crate) failures: u32,
    pub(crate) duration_seconds: f64,
    /// Its first `<failure>` or `<error>`.
    pub(crate) first_failure: Option<Failure>,
    /// The text of its `<system-out>` and `<system-err>` elements, where a
    /// test prints its seed.
    pub(crate) output: String,
}

/// A `<failure>` or an `<error>` element.
pub(crate) struct Failure {
    /// Its `message` attribute, which runners fill with the failure's first
    /// line.
    pub(crate) message: Option<String>,
    /// Its text: the runner's report of the failure.
    pub(crate) text: String,
}

/// The runner that wrote a JUnit file, from its name: the part before the
/// first `-` or `.` (`nextest-ci.xml`, `vitest.xml`).
pub(crate) fn runner(file_name: &str) -> &str {
    let stem = file_name.strip_suffix(".xml").unwrap_or(file_name);
    stem.split(['-', '.']).next().unwrap_or(stem)
}

/// The runtime a JUnit file's name adds after its runner (`vitest-node24.xml`,
/// `pytest-3.14t.xml`); none for nextest, whose suffix names a profile or a
/// cell, never a runtime.
pub(crate) fn file_runtime(file_name: &str) -> Option<&str> {
    let stem = file_name.strip_suffix(".xml").unwrap_or(file_name);
    let runner = runner(file_name);
    if runner == "nextest" {
        return None;
    }
    stem.strip_prefix(runner)?
        .strip_prefix('-')
        .filter(|runtime| !runtime.is_empty())
}

/// Where the text being read goes.
#[derive(Clone, Copy)]
enum Capture {
    FirstFailure,
    Output,
}

/// Every test case of `text`, in document order.
pub(crate) fn test_cases(runner: &str, text: &str) -> Result<Vec<TestCase>> {
    let mut reader = Reader::from_str(text);
    let mut suite = String::new();
    let mut has_suite = false;
    let mut cases = Vec::new();
    let mut open: Option<TestCase> = None;
    let mut capture: Option<Capture> = None;
    // The elements open where the reader is: a file a killed runner left
    // unfinished ends with some still open, which quick-xml accepts.
    let mut depth = 0_usize;
    loop {
        let event = reader.read_event().context("parsing the JUnit XML")?;
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        match event {
            Event::Start(element) | Event::Empty(element)
                if matches!(element.local_name().as_ref(), b"testsuite" | b"testsuites") =>
            {
                has_suite = true;
                if element.local_name().as_ref() == b"testsuite" {
                    suite = attribute(&element, "name")?.unwrap_or_default();
                }
            }
            Event::Start(element) if element.local_name().as_ref() == b"testcase" => {
                open = Some(test_case(runner, &suite, &element)?);
            }
            Event::Empty(element) if element.local_name().as_ref() == b"testcase" => {
                cases.push(test_case(runner, &suite, &element)?);
            }
            Event::Start(element) => {
                if let Some(case) = open.as_mut() {
                    capture = child(case, &element)?;
                }
            }
            Event::Empty(element) => {
                if let Some(case) = open.as_mut() {
                    child(case, &element)?;
                }
            }
            Event::End(element) if element.local_name().as_ref() == b"testcase" => {
                cases.extend(open.take());
                capture = None;
            }
            Event::End(_) => capture = None,
            Event::Text(text) => {
                append(
                    open.as_mut(),
                    capture,
                    &text.xml_content(XmlVersion::Implicit1_0)?,
                );
            }
            Event::CData(data) => {
                append(
                    open.as_mut(),
                    capture,
                    &data.xml_content(XmlVersion::Implicit1_0)?,
                );
            }
            Event::GeneralRef(reference) => {
                append(open.as_mut(), capture, &resolve(&reference)?);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth > 0 {
        bail!("the file ends inside an element");
    }
    if !has_suite {
        bail!("the file holds no <testsuites> or <testsuite> element");
    }
    Ok(cases)
}

/// How many test cases the file's `<testsuites>` element declares, its
/// `tests` attribute; none when it has no such element or attribute.
pub(crate) fn declared_tests(text: &str) -> Result<Option<usize>> {
    let mut reader = Reader::from_str(text);
    loop {
        match reader.read_event().context("parsing the JUnit XML")? {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"testsuites" =>
            {
                return attribute(&element, "tests")?
                    .map(|tests| {
                        tests
                            .parse()
                            .with_context(|| format!("<testsuites> declares `{tests}` tests"))
                    })
                    .transpose();
            }
            Event::Start(_) | Event::Empty(_) | Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Records a child element of an open test case, and says where its text
/// goes.
fn child(case: &mut TestCase, element: &BytesStart<'_>) -> Result<Option<Capture>> {
    Ok(match element.local_name().as_ref() {
        b"failure" | b"error" => {
            case.ending = Ending::Failed;
            case.failures += 1;
            if case.first_failure.is_some() {
                None
            } else {
                case.first_failure = Some(Failure {
                    message: attribute(element, "message")?,
                    text: String::new(),
                });
                Some(Capture::FirstFailure)
            }
        }
        b"skipped" => {
            if case.ending == Ending::Passed {
                case.ending = Ending::Skipped;
            }
            None
        }
        b"system-out" | b"system-err" => Some(Capture::Output),
        _ => None,
    })
}

fn append(case: Option<&mut TestCase>, capture: Option<Capture>, text: &str) {
    let Some(case) = case else { return };
    match capture {
        Some(Capture::FirstFailure) => {
            if let Some(failure) = case.first_failure.as_mut() {
                failure.text.push_str(text);
            }
        }
        Some(Capture::Output) => case.output.push_str(text),
        None => {}
    }
}

/// The text an entity or character reference stands for.
fn resolve(reference: &BytesRef<'_>) -> Result<String> {
    if let Some(character) = reference.resolve_char_ref()? {
        return Ok(character.to_string());
    }
    let name = reference.decode()?;
    match resolve_predefined_entity(&name) {
        Some(text) => Ok(text.to_owned()),
        None => bail!("the file uses the undeclared entity `&{name};`"),
    }
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
        failures: 0,
        duration_seconds,
        first_failure: None,
        output: String::new(),
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
            return Ok(Some(
                attribute
                    .normalized_value(XmlVersion::Implicit1_0)?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{file_runtime, pytest_node_id};

    #[rstest]
    #[case::module_function("tests.test_page", "test_title", "tests/test_page.py::test_title")]
    #[case::class_method(
        "tests.test_page.TestTitle",
        "test_shown",
        "tests/test_page.py::TestTitle::test_shown"
    )]
    #[case::nested_classes(
        "tests.test_page.TestTitle.TestEmpty",
        "test_shown",
        "tests/test_page.py::TestTitle::TestEmpty::test_shown"
    )]
    fn pytest_class_name_gives_its_node_id(
        #[case] class: &str,
        #[case] name: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(pytest_node_id(class, name), expected);
    }

    #[rstest]
    #[case::vitest_node_line("vitest-node24.xml", Some("node24"))]
    #[case::pytest_free_threaded("pytest-3.14t.xml", Some("3.14t"))]
    #[case::no_suffix("playwright.xml", None)]
    #[case::nextest_profile("nextest-ci.xml", None)]
    fn junit_file_name_gives_its_runtime(#[case] file: &str, #[case] expected: Option<&str>) {
        assert_eq!(file_runtime(file), expected);
    }
}
