//! The CLI's output writer (STYLE.md): one of the three writers of stdout
//! and stderr, for a command's human output and its `--json` output on
//! stdout, and its warnings, hints, progress and failures on stderr, so
//! that stdout under `--json` holds one JSON value and nothing else (ADR
//! 0069).
//!
//! Human output (ADR 0073) is English with `.` decimals and ISO 8601, and
//! reads no locale; [`Human`] shows times in the local zone with their
//! offset, colours only where `--color` and `NO_COLOR` allow it, a stream
//! at a time, so no escape byte reaches a pipe, and shows progress on
//! stderr only when stderr is a terminal. Colour and progress never carry
//! meaning alone: with both gone the text says the same.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{IsTerminal as _, Write as _};

use goodfetch_protocol::cli::CommandFailed;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

fn write_stdout(bytes: &[u8]) {
    #[expect(
        clippy::disallowed_methods,
        reason = "the CLI's output writer is one of the three writers of stdout (STYLE.md)"
    )]
    let mut stdout = std::io::stdout().lock();
    // A closed stdout, such as a pipe whose reader left, ends the output.
    let _ = stdout.write_all(bytes).and_then(|()| stdout.flush());
}

fn write_stderr(bytes: &[u8]) {
    #[expect(
        clippy::disallowed_methods,
        reason = "the CLI's output writer is one of the three writers of stderr (STYLE.md)"
    )]
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(bytes).and_then(|()| stderr.flush());
}

/// Writes `text` and a newline on stdout.
pub fn line(text: &str) {
    write_stdout(format!("{text}\n").as_bytes());
}

/// Writes `bytes` on stdout as they are, such as a file's that a command
/// streams there.
pub fn bytes(bytes: &[u8]) {
    write_stdout(bytes);
}

/// Writes `value` on stdout as one line of JSON.
///
/// # Panics
///
/// If `value` does not serialize, which no output type of
/// `goodfetch-protocol` does.
pub fn json(value: &impl Serialize) {
    let mut bytes = serde_json::to_vec(value).expect("invariant: the output types serialize");
    bytes.push(b'\n');
    write_stdout(&bytes);
}

/// Writes `text` and a newline on stderr: a warning, a hint or a failure,
/// which never reaches stdout.
pub fn note(text: &str) {
    write_stderr(format!("{text}\n").as_bytes());
}

/// Prints a command's failure: under `--json` the [`CommandFailed`] on
/// stdout, else as `human` shows it on stderr, its message, then `code`,
/// `retryable` and its `fix` when it has one, each on a line of its own.
pub fn failed(failed: &CommandFailed, json: bool, human: &Human) {
    if json {
        self::json(failed);
        return;
    }
    let error = &failed.error;
    let mut details = vec![
        ("code", error.code.clone()),
        ("retryable", error.retryable.to_string()),
    ];
    if let Some(fix) = error.details.get("fix").and_then(serde_json::Value::as_str) {
        details.push(("fix", fix.to_owned()));
    }
    human.failure(&error.message, &details);
}

/// When human output is coloured: `--color auto|always|never`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum ColourChoice {
    // Plain comments, not doc comments, so that `--help` lists the three
    // values on one line.
    // On a terminal, unless `NO_COLOR` is set and not empty.
    #[default]
    Auto,
    // Always, `NO_COLOR` and pipes included.
    Always,
    // Never.
    Never,
}

// The `--color` flag, which every command of the command line takes,
// before or after its subcommand. Plain comments, not doc comments, which
// would become the help of every command that flattens it.
//
// Each command group of the root carries it itself, global within the
// group, so clap copies it into the group's commands only when it builds
// that group: a flag global at the root would be copied into every group
// whenever the command line is parsed, `--version` included (ADR 0071's
// startup figure). The root takes it as a flag of its own, not global.
#[derive(Clone, Copy, Debug, Default, clap::Args)]
pub struct ColourFlag {
    /// When to colour human output; `auto` colours on a terminal unless
    /// `NO_COLOR` is set and not empty.
    #[arg(
        long,
        global = true,
        value_enum,
        default_value_t,
        value_name = "WHEN",
        display_order = ColourFlag::DISPLAY_ORDER
    )]
    pub color: ColourChoice,
}

impl ColourFlag {
    /// The flag's id in a parse.
    pub const ID: &str = "color";

    /// Where `--help` sorts the flag among a command's own flags: where it
    /// sorted while it was global at the root, after the root's six
    /// hand-written commands, so every command's help reads as it did.
    const DISPLAY_ORDER: usize = 6;
}

/// The variable that turns colour off when it is set and not empty
/// (no-color.org).
const NO_COLOR: &str = "NO_COLOR";

/// The variable that names the local zone.
const TZ: &str = "TZ";

/// A style of human output, as the SGR sequence that starts it.
#[derive(Clone, Copy)]
enum Style {
    /// A name or a label.
    Strong,
    /// A failure's message.
    Failure,
    /// A warning's label.
    Warning,
}

impl Style {
    const RESET: &str = "\x1b[0m";

    fn start(self) -> &'static str {
        match self {
            Self::Strong => "\x1b[1m",
            Self::Failure => "\x1b[1;31m",
            Self::Warning => "\x1b[1;33m",
        }
    }
}

/// How human output shows here: the local zone, and whether stdout and
/// stderr are coloured and progress shown.
pub struct Human {
    zone: TimeZone,
    colour_stdout: bool,
    colour_stderr: bool,
    progress: bool,
}

impl Human {
    /// Human output under `choice`, the executable's `environment` and its
    /// streams: each stream coloured on its own terminal (or always under
    /// `--color always`), progress on stderr only when it is a terminal,
    /// and times in the zone `TZ` names, else the system's.
    pub fn new(choice: ColourChoice, environment: &BTreeMap<OsString, OsString>) -> Self {
        #[expect(
            clippy::disallowed_methods,
            reason = "the CLI's output writer asks whether its own streams are terminals"
        )]
        let (stdout_terminal, stderr_terminal) = (
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        );
        let no_color = environment
            .get(OsStr::new(NO_COLOR))
            .is_some_and(|value| !value.is_empty());
        let coloured = |terminal: bool| match choice {
            ColourChoice::Always => true,
            ColourChoice::Never => false,
            ColourChoice::Auto => terminal && !no_color,
        };
        Self {
            zone: local_zone(environment),
            colour_stdout: coloured(stdout_terminal),
            colour_stderr: coloured(stderr_terminal),
            progress: stderr_terminal,
        }
    }

    /// Writes `name: value` on stdout, the name styled.
    pub fn field(&self, name: &str, value: &str) {
        let name = styled(self.colour_stdout, Style::Strong, &format!("{name}:"));
        write_stdout(format!("{name} {value}\n").as_bytes());
    }

    /// Whether a person can be asked: stdin is a terminal. A person-only
    /// command checks it before any engine call, so it is refused at once
    /// under a script, an agent or CI (ADRs 0041 and 0069).
    pub fn can_ask(&self) -> bool {
        #[expect(
            clippy::disallowed_methods,
            reason = "a person-only prompt asks whether stdin is a terminal before it asks"
        )]
        std::io::stdin().is_terminal()
    }

    /// Asks a person `question` on stderr and reads their answer, a line of
    /// stdin, without its ends' white space; none when stdin is not a
    /// terminal, where no person answers: a person-only prompt is then
    /// refused (ADRs 0041 and 0069). An ended stdin answers nothing.
    pub fn ask(&self, question: &str) -> Option<String> {
        if !self.can_ask() {
            return None;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "a person-only prompt reads the person's answer from stdin"
        )]
        let stdin = std::io::stdin();
        let question = styled(self.colour_stderr, Style::Strong, question);
        write_stderr(format!("{question} ").as_bytes());
        let mut answer = String::new();
        let _ = stdin.read_line(&mut answer);
        Some(answer.trim().to_owned())
    }

    /// Writes `warning: text` on stderr, the label styled.
    pub fn warning(&self, text: &str) {
        let label = styled(self.colour_stderr, Style::Warning, "warning:");
        write_stderr(format!("{label} {text}\n").as_bytes());
    }

    /// Writes a failure on stderr: its message, then each `label: value`
    /// on a line of its own.
    pub fn failure(&self, message: &str, details: &[(&str, String)]) {
        let mut lines = vec![styled(self.colour_stderr, Style::Failure, message)];
        lines.extend(details.iter().map(|(label, value)| {
            let label = styled(self.colour_stderr, Style::Strong, &format!("{label}:"));
            format!("{label} {value}")
        }));
        write_stderr(format!("{}\n", lines.join("\n")).as_bytes());
    }

    /// `rfc3339`, a point in time, in ISO 8601 in the local zone with its
    /// offset; `None` when it is no point in time.
    pub fn time(&self, rfc3339: &str) -> Option<String> {
        let instant: Timestamp = rfc3339.parse().ok()?;
        Some(
            instant
                .display_with_offset(self.zone.to_offset(instant))
                .to_string(),
        )
    }

    /// Shows `text` as progress on stderr while the [`Progress`] lives,
    /// when stderr is a terminal; nothing otherwise.
    pub fn progress(&self, text: &str) -> Progress {
        if self.progress {
            write_stderr(format!("{text}…").as_bytes());
        }
        Progress {
            shown: self.progress,
        }
    }
}

/// Progress shown on stderr, erased when it drops.
pub struct Progress {
    shown: bool,
}

impl Drop for Progress {
    fn drop(&mut self) {
        if self.shown {
            // Back to the line's start, then erase it.
            write_stderr(b"\r\x1b[2K");
        }
    }
}

fn styled(coloured: bool, style: Style, text: &str) -> String {
    if coloured {
        format!("{}{text}{}", style.start(), Style::RESET)
    } else {
        text.to_owned()
    }
}

/// The zone `TZ` names (an IANA name, with or without a leading `:`, or a
/// POSIX rule), UTC when it is empty or names none, as the C library
/// reads it, and the system's when `TZ` is unset.
fn local_zone(environment: &BTreeMap<OsString, OsString>) -> TimeZone {
    let Some(value) = environment.get(OsStr::new(TZ)) else {
        return TimeZone::try_system().unwrap_or(TimeZone::UTC);
    };
    let Some(value) = value.to_str() else {
        return TimeZone::UTC;
    };
    let name = value.strip_prefix(':').unwrap_or(value);
    if name.is_empty() {
        return TimeZone::UTC;
    }
    TimeZone::get(name)
        .or_else(|_| TimeZone::posix(name))
        .unwrap_or_else(|error| {
            tracing::warn!(%error, zone = name, "TZ names no zone; times show in UTC");
            TimeZone::UTC
        })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{ColourChoice, Human};

    #[rstest]
    #[case::iana_name("Asia/Kathmandu", Some("2026-01-05T14:45:00+05:45"))]
    #[case::iana_name_after_a_colon(":Asia/Kathmandu", Some("2026-01-05T14:45:00+05:45"))]
    #[case::posix_rule("NPT-5:45", Some("2026-01-05T14:45:00+05:45"))]
    #[case::daylight_saving_time("America/St_Johns", Some("2026-01-05T05:30:00-03:30"))]
    #[case::empty("", Some("2026-01-05T09:00:00+00:00"))]
    #[case::naming_no_zone("Nowhere/Atlantis", Some("2026-01-05T09:00:00+00:00"))]
    fn a_point_in_time_shows_in_the_zone_tz_names_with_its_offset(
        #[case] tz: &str,
        #[case] shown: Option<&str>,
    ) {
        let human = Human::new(ColourChoice::Never, &[("TZ".into(), tz.into())].into());

        let time = human.time("2026-01-05T09:00:00Z");

        assert_eq!(time.as_deref(), shown);
    }

    #[test]
    fn a_text_that_is_no_point_in_time_shows_as_none() {
        let human = Human::new(ColourChoice::Never, &[("TZ".into(), "UTC".into())].into());

        let time = human.time("first");

        assert_eq!(time, None);
    }
}
