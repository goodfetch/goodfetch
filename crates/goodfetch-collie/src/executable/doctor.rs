//! `doctor` (ADR 0043, spec #608), a command of both Products'
//! executables: it checks the installation, Goodfetch home, the installed
//! executables, the Daemon of this executable's major, the version that
//! Daemon runs and the browser build a Run launches by default, and
//! prints one finding for each, with a fix for each that is not in order.
//! It then writes the bug-report file for the user to attach to an issue
//! (spec #564), which it sends nowhere. It starts nothing, downloads
//! nothing and exits `0` whatever it finds; its `--json` output is
//! `goodfetch-protocol`'s `DoctorReport`.
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI checks the machine's files before and outside any async worker (ADR 0060)"
)]

use std::path::Path;
use std::process::ExitCode;

use goodfetch_engine::browser_cache::{
    BrowserChoice, BrowserSource, CHROME_HEADLESS_SHELL, Pins, Platform, browser_source,
    entry_present,
};
use goodfetch_engine::settings::{Resolved, get_or_default};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::daemon::DaemonState;
use goodfetch_protocol::doctor::{
    BugReportFile, DoctorFinding, DoctorReport, FindingId, FindingStatus,
};
use goodfetch_protocol::egress::EGRESS_OFFLINE;
use goodfetch_protocol::jsonrpc::ErrorObject;
use goodfetch_protocol::page::BROWSER_HEADLESS_SHELL;
use serde_json::Value;

use super::daemon::{Failure, Machine};
use super::output;

mod bug_report;

/// `doctor`: checks the installation `machine` runs in, writes the
/// bug-report file, each URL in it whole under `include_urls`, and prints
/// what it found, under `json` as one `DoctorReport`.
pub(super) fn doctor(machine: &Machine<'_>, json: bool, include_urls: bool) -> ExitCode {
    let urls = if include_urls {
        bug_report::Urls::Whole
    } else {
        bug_report::Urls::Reduced
    };
    let report = report(machine, urls);
    if json {
        output::json(&report);
    } else {
        for finding in &report.findings {
            output::line(&format!(
                "{} {}: {}",
                status_name(finding.status),
                id_name(finding.id),
                finding.message
            ));
            if let Some(fix) = &finding.fix {
                output::line(&format!("  fix: {fix}"));
            }
        }
        output::line(&match &report.bug_report {
            BugReportFile::Written { path } => format!(
                "bug report: written to {path}, for you to attach to an issue; doctor sent nothing"
            ),
            BugReportFile::NotWritten { message } => format!("bug report: not written: {message}"),
        });
    }
    ExitCode::SUCCESS
}

/// The findings about the installation `machine` runs in, and the
/// bug-report file, each URL in it as `urls` says.
fn report(machine: &Machine<'_>, urls: bug_report::Urls) -> DoctorReport {
    let executable = machine.identity.product.executable_name();
    let settings = machine.settings().map_err(Failure::into_error);
    let (daemon, version_skew) = daemon(machine, executable);
    let findings = vec![
        home(machine.home.root(), settings.as_ref().err()),
        executables(machine),
        daemon,
        version_skew,
        browsers(machine, settings.ok(), executable),
    ];
    let bug_report = bug_report::write(machine, &findings, urls);
    DoctorReport {
        findings,
        bug_report,
    }
}

/// A finding in order.
fn ok(id: FindingId, message: String) -> DoctorFinding {
    DoctorFinding {
        id,
        status: FindingStatus::Ok,
        message,
        fix: None,
    }
}

/// A finding not in order, `status`, with its `fix`.
fn not_ok(id: FindingId, status: FindingStatus, message: String, fix: String) -> DoctorFinding {
    DoctorFinding {
        id,
        status,
        message,
        fix: Some(fix),
    }
}

/// The `fix` a failure's error object holds, else `otherwise`.
fn fix_of(error: &ErrorObject, otherwise: String) -> String {
    error
        .data
        .details
        .get("fix")
        .and_then(Value::as_str)
        .map_or(otherwise, str::to_owned)
}

/// Whether Goodfetch home at `root` can be written, and its settings read:
/// `unreadable` is why they cannot be.
fn home(root: &Path, unreadable: Option<&ErrorObject>) -> DoctorFinding {
    let writable = std::fs::create_dir_all(root).is_ok()
        && tempfile::Builder::new()
            .prefix(".write-check")
            .tempfile_in(root)
            .is_ok();
    if !writable {
        return not_ok(
            FindingId::Home,
            FindingStatus::Failing,
            format!("Goodfetch home at {} cannot be written", root.display()),
            "Let this user write the folder, or point GOODFETCH_HOME at one it can write"
                .to_owned(),
        );
    }
    match unreadable {
        Some(error) => not_ok(
            FindingId::Home,
            FindingStatus::Failing,
            error.message.clone(),
            fix_of(
                error,
                "Correct the Operator policy or user configuration it names".to_owned(),
            ),
        ),
        None => ok(
            FindingId::Home,
            format!(
                "Goodfetch home at {} can be written, and its settings read",
                root.display()
            ),
        ),
    }
}

/// Whether this executable is recorded in the list of installed
/// executables, which keeps the builds it pins (ADR 0098).
fn executables(machine: &Machine<'_>) -> DoctorFinding {
    let fix = "Let this user write Goodfetch home, then run any command again".to_owned();
    let entries = match machine.list().read() {
        Ok(entries) => entries,
        Err(error) => {
            return not_ok(
                FindingId::Executables,
                FindingStatus::Failing,
                format!("the list of installed executables cannot be read: {error}"),
                fix,
            );
        }
    };
    let identity = machine.identity;
    let recorded = machine.executable.as_ref().is_some_and(|path| {
        let path = path.display().to_string();
        entries.iter().any(|entry| {
            entry.path == path
                && entry.product == identity.product
                && entry.product_version == identity.product_version
        })
    });
    let others = entries.len().saturating_sub(usize::from(recorded));
    if recorded {
        ok(
            FindingId::Executables,
            format!(
                "{} {} is recorded among the installed executables, {others} others beside it",
                identity.product.name(),
                identity.product_version
            ),
        )
    } else {
        not_ok(
            FindingId::Executables,
            FindingStatus::Warning,
            format!(
                "{} {} is not recorded among the installed executables, so `browser clean` may delete the builds it pins",
                identity.product.name(),
                identity.product_version
            ),
            fix,
        )
    }
}

/// Whether the Daemon of this executable's major answers, and whether it
/// runs this executable's version.
fn daemon(machine: &Machine<'_>, executable: &str) -> (DoctorFinding, DoctorFinding) {
    let major = machine.major();
    let restart = format!("Run `{executable} daemon restart`");
    let answered = machine
        .connect(false, Product::Collie)
        .and_then(|connected| {
            connected
                .map(|mut connected| {
                    let status =
                        connected.call::<DaemonState>("daemon.status", &serde_json::json!({}));
                    connected.close();
                    status
                })
                .transpose()
        });
    match answered {
        Ok(None) => (
            ok(
                FindingId::Daemon,
                format!(
                    "No Daemon of major {major} runs; the first command that needs one starts it"
                ),
            ),
            ok(
                FindingId::VersionSkew,
                "No Daemon runs another version than this executable".to_owned(),
            ),
        ),
        Ok(Some(state)) => {
            let running = format!("{} {}", state.product.name(), state.product_version);
            let skew = if state.product_version == machine.identity.product_version {
                ok(
                    FindingId::VersionSkew,
                    format!("The Daemon runs this executable's version, {running}"),
                )
            } else {
                not_ok(
                    FindingId::VersionSkew,
                    FindingStatus::Warning,
                    format!(
                        "The Daemon runs {running}, and this executable is {} {}",
                        machine.identity.product.name(),
                        machine.identity.product_version
                    ),
                    format!(
                        "{restart} once its live Runs end: it starts the Daemon from the newest installed executable of major {major}"
                    ),
                )
            };
            (
                ok(
                    FindingId::Daemon,
                    format!(
                        "The Daemon of major {major} runs {running}, with {} live Runs",
                        state.live_runs
                    ),
                ),
                skew,
            )
        }
        Err(Failure::DaemonRefused(error)) => {
            let fix = fix_of(&error, restart);
            (
                not_ok(
                    FindingId::Daemon,
                    FindingStatus::Failing,
                    error.message.clone(),
                    fix.clone(),
                ),
                not_ok(
                    FindingId::VersionSkew,
                    FindingStatus::Failing,
                    format!(
                        "The Daemon refuses this executable's version: {}",
                        error.message
                    ),
                    fix,
                ),
            )
        }
        Err(failure) => {
            let error = failure.into_error();
            (
                not_ok(
                    FindingId::Daemon,
                    FindingStatus::Failing,
                    error.message.clone(),
                    fix_of(&error, restart.clone()),
                ),
                not_ok(
                    FindingId::VersionSkew,
                    FindingStatus::Warning,
                    "The Daemon did not answer, so its version is not known".to_owned(),
                    restart,
                ),
            )
        }
    }
}

/// Whether the browser build a Run launches by default, under `settings`
/// (the defaults where they cannot be read), is where the launch finds it.
fn browsers(machine: &Machine<'_>, settings: Option<&Resolved>, executable: &str) -> DoctorFinding {
    let shell = get_or_default::<bool>(settings, &BROWSER_HEADLESS_SHELL);
    let offline = get_or_default::<bool>(settings, &EGRESS_OFFLINE);
    let (name, platform) = match browser_source(
        Platform::current(),
        &BrowserChoice::Default,
        machine.environment,
    ) {
        BrowserSource::Pinned { name, platform, .. } => {
            (if shell { CHROME_HEADLESS_SHELL } else { name }, platform)
        }
        BrowserSource::Installed { candidates, .. } => {
            return match candidates.iter().find(|candidate| candidate.exists()) {
                Some(found) => ok(
                    FindingId::Browsers,
                    format!("Runs launch the installed browser at {}", found.display()),
                ),
                None => not_ok(
                    FindingId::Browsers,
                    FindingStatus::Failing,
                    "No installed browser is where Runs launch one on this platform".to_owned(),
                    "Install the distribution's Chromium".to_owned(),
                ),
            };
        }
    };
    let pins = Pins {
        pinned_by: PinnedBy {
            product: machine.identity.product,
            product_version: machine.identity.product_version.clone(),
        },
        artifacts: crate::pinned_artifacts(),
    };
    let Some(pinned) = pins.browser(name, platform) else {
        return not_ok(
            FindingId::Browsers,
            FindingStatus::Failing,
            format!("This version pins no {name} build for this platform"),
            format!("Set `{}: false`", BROWSER_HEADLESS_SHELL.name),
        );
    };
    let location = machine.cache_location();
    let root = location.root();
    let build = format!("The {name} build this version pins");
    if entry_present(root, &pinned.id) {
        return ok(
            FindingId::Browsers,
            format!("{build} is in the Browser cache at {}", root.display()),
        );
    }
    let install = if name == CHROME_HEADLESS_SHELL {
        format!("{executable} browser install {CHROME_HEADLESS_SHELL}")
    } else {
        format!("{executable} browser install")
    };
    if offline {
        not_ok(
            FindingId::Browsers,
            FindingStatus::Failing,
            format!(
                "{build} is not in the Browser cache at {}, and `{}` forbids downloading it",
                root.display(),
                EGRESS_OFFLINE.name
            ),
            format!(
                "Run `{install}` where the network can be reached, point GOODFETCH_BROWSERS_PATH at a cache that holds it, or lift `{}`",
                EGRESS_OFFLINE.name
            ),
        )
    } else {
        not_ok(
            FindingId::Browsers,
            FindingStatus::Warning,
            format!(
                "{build} is not in the Browser cache at {}: the first Run downloads it",
                root.display()
            ),
            format!("Run `{install}` to download it beforehand"),
        )
    }
}

/// How `doctor` names `status` for people.
fn status_name(status: FindingStatus) -> &'static str {
    match status {
        FindingStatus::Ok => "ok",
        FindingStatus::Warning => "warning",
        FindingStatus::Failing => "failing",
    }
}

/// How `doctor` names `id` for people: its `--json` name.
fn id_name(id: FindingId) -> &'static str {
    match id {
        FindingId::Home => "home",
        FindingId::Executables => "executables",
        FindingId::Daemon => "daemon",
        FindingId::VersionSkew => "version_skew",
        FindingId::Browsers => "browsers",
    }
}
