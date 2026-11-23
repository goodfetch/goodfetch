//! Conformance runs' Live check results as JUnit (ADR 0064; TESTING.md,
//! Live checks): a run against the online Proving ground, such as the daily
//! channel's gate run, is a Live check, and it reports to the results store
//! as a test does, through a JUnit file its job uploads under the `live`
//! tier. Each document becomes one test case, named
//! `<suite> <target>[ gate]` under the class `conformance`, which fails
//! when a case of the document failed, is skipped, which the ingest stores
//! as inconclusive, when none failed but one was inconclusive or not
//! reached, and passes otherwise.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::result_document::{
    CapabilityResult, CaseOutcome, CaseResult, ConformanceDocument, LoadedDocument, read,
};
use quick_xml::escape::escape;

/// The class of every Live check's test case: with its name, its ID, as
/// the ingest names a test of a runner it does not know.
const CLASS: &str = "conformance";

/// Reads each `conformance` document of `documents`, which the reader
/// accepts only with every outcome it stores recomputed, and writes their
/// Live checks' results to the JUnit file `junit`.
pub(crate) fn write(documents: &[PathBuf], junit: &Path) -> Result<()> {
    let mut cases = Vec::with_capacity(documents.len());
    for path in documents {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        match read(&bytes).with_context(|| format!("reading {}", path.display()))? {
            LoadedDocument::Conformance(document) => cases.push(LiveCheck::of(&document)),
            LoadedDocument::Journey(_) => bail!("{} is no `conformance` document", path.display()),
        }
    }
    let failures = cases.iter().filter(|case| case.failed.is_some()).count();
    let skipped = cases
        .iter()
        .filter(|case| case.failed.is_none() && case.inconclusive.is_some())
        .count();
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let counts = format!(
        "tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\"",
        cases.len()
    );
    let _ = writeln!(xml, "<testsuites name=\"{CLASS}\" {counts}>");
    let _ = writeln!(xml, "  <testsuite name=\"{CLASS}\" {counts}>");
    for case in &cases {
        case.write(&mut xml);
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    if let Some(folder) = junit.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(junit, xml).with_context(|| format!("writing {}", junit.display()))
}

/// One document's Live check.
struct LiveCheck {
    name: String,
    /// Why it failed: the cases that did.
    failed: Option<String>,
    /// Why it is inconclusive: the cases neither passed nor failed.
    inconclusive: Option<String>,
    /// The run and each case's outcome.
    output: String,
}

impl LiveCheck {
    fn of(document: &ConformanceDocument) -> Self {
        let header = document.header();
        let cases: Vec<&CaseResult> = document
            .capabilities()
            .iter()
            .flat_map(CapabilityResult::cases)
            .collect();
        let named = |outcomes: &[CaseOutcome], what: &str| {
            let ids: Vec<&str> = cases
                .iter()
                .filter(|case| outcomes.contains(&case.outcome()))
                .map(|case| case.facts().id.as_str())
                .collect();
            (!ids.is_empty()).then(|| format!("{what}: {}", ids.join(", ")))
        };
        let mut output = format!(
            "suite {} {}, Proving ground {} seed {}\n",
            header.suite.name,
            header.suite.version,
            header.proving_ground.version,
            header.proving_ground.seed
        );
        for case in &cases {
            let _ = writeln!(output, "{} {}", case.outcome().as_str(), case.facts().id);
        }
        Self {
            name: format!(
                "{} {}{}",
                header.suite.name,
                header.target.name,
                if header.gate { " gate" } else { "" }
            ),
            failed: named(&[CaseOutcome::Failed], "failed"),
            inconclusive: named(
                &[CaseOutcome::Inconclusive, CaseOutcome::NotReached],
                "inconclusive",
            ),
            output,
        }
    }

    fn write(&self, xml: &mut String) {
        let _ = writeln!(
            xml,
            "    <testcase classname=\"{CLASS}\" name=\"{}\" time=\"0\">",
            escape(&self.name)
        );
        if let Some(message) = &self.failed {
            let _ = writeln!(
                xml,
                "      <failure message=\"{}\">{}</failure>",
                escape(message),
                escape(message)
            );
        } else if let Some(message) = &self.inconclusive {
            let _ = writeln!(xml, "      <skipped message=\"{}\"/>", escape(message));
        }
        let _ = writeln!(
            xml,
            "      <system-out>{}</system-out>",
            escape(&self.output)
        );
        xml.push_str("    </testcase>\n");
    }
}
