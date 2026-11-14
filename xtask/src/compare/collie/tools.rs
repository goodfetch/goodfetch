//! How each tool of the Collie scenario's six cells is run (issues 2282,
//! 2441, 2442, 2443 and 2444): the command that starts its driver, with
//! none of xtask's own Goodfetch variables, the flags its browser takes
//! past every tool's, and the cells it runs no trial of.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::trials::{Contender, Exercise};

/// The executables and files a trial takes.
pub(crate) struct Tools<'a> {
    pub(crate) proving_ground: &'a Path,
    /// Collie's Attached engine.
    pub(crate) engine: &'a Path,
    /// `xtask/compare-collie/act.ts`, which Node.js runs for Collie and
    /// Playwright, as an absolute path.
    pub(crate) driver: PathBuf,
    /// `xtask/compare-collie-arms/act.ts`, which it runs for the arms but
    /// chromiumoxide (issues 2441, 2442 and 2443), as an absolute path.
    pub(crate) arms_driver: PathBuf,
    /// chromiumoxide's side's executable
    /// ([`super::super::run::chromiumoxide_executable`], issue 2444).
    pub(crate) chromiumoxide: PathBuf,
    /// Chrome for Testing's folder.
    pub(crate) browser: &'a Path,
    /// xtask's own `GOODFETCH_*` variables, which the tool runs without.
    pub(crate) goodfetch_variables: &'a [OsString],
}

impl Tools<'_> {
    /// The driver that runs `tool`, none of this process's Goodfetch
    /// variables in its environment: Node.js running its script, told which
    /// tool by `--tool`, or chromiumoxide's side's own executable, which
    /// runs that one tool.
    pub(crate) fn command(&self, tool: Contender) -> Command {
        let script = match tool {
            Contender::Collie | Contender::CollieProgram | Contender::Playwright => &self.driver,
            Contender::PuppeteerLocator
            | Contender::PuppeteerPage
            | Contender::Stagehand
            | Contender::AgentBrowser => &self.arms_driver,
            Contender::Chromiumoxide => {
                return without_goodfetch_variables(self, &self.chromiumoxide);
            }
        };
        let mut command = node(self, script);
        command.args(["--tool", tool.name()]);
        command
    }
}

/// The flags `tool`'s browser is launched with past every tool's: Stagehand
/// drives it through an extension that speaks CDP over a WebSocket of its
/// own, which Chrome opens to it only with these (issue 2441).
pub(crate) fn browser_flags(tool: Contender) -> &'static [&'static str] {
    match tool {
        Contender::Stagehand => &[
            "--enable-unsafe-extension-debugging",
            "--remote-allow-origins=*",
        ],
        Contender::Collie
        | Contender::CollieProgram
        | Contender::Playwright
        | Contender::PuppeteerLocator
        | Contender::PuppeteerPage
        | Contender::AgentBrowser
        | Contender::Chromiumoxide => &[],
    }
}

/// Why `tool` runs no trial of `exercise`, when it runs none: Stagehand
/// and agent-browser open no Session of their own (issues 2441 and 2442).
pub(crate) fn skipped(tool: Contender, exercise: Exercise) -> Option<&'static str> {
    if exercise != Exercise::Sessions {
        return None;
    }
    match tool {
        Contender::Stagehand => {
            Some("Stagehand drives the browser's one context, so it opens no Session")
        }
        // Its instances share the default context over `--cdp`; its binary
        // creates browser contexts but never disposes of one.
        Contender::AgentBrowser => Some(
            "agent-browser has no command that closes a context it opened, so it opens no \
             Session",
        ),
        Contender::Collie
        | Contender::CollieProgram
        | Contender::Playwright
        | Contender::PuppeteerLocator
        | Contender::PuppeteerPage
        | Contender::Chromiumoxide => None,
    }
}

/// Node.js running the driver `script`, none of this process's Goodfetch
/// variables in its environment.
pub(super) fn node(tools: &Tools<'_>, script: &Path) -> Command {
    let mut command = without_goodfetch_variables(tools, Path::new("node"));
    command.arg(script);
    command
}

/// `program`, none of this process's Goodfetch variables in its
/// environment.
fn without_goodfetch_variables(tools: &Tools<'_>, program: &Path) -> Command {
    let mut command = Command::new(program);
    for name in tools.goodfetch_variables {
        command.env_remove(name);
    }
    command
}

/// Makes the process `command` starts adopt what its descendants leave
/// behind, such as the daemon agent-browser's command starts in a session
/// of its own and leaves when it exits: Linux reparents each to it rather
/// than to init, so they stay in its tree, in its figures and in
/// [`super::super::trial::kill_tree`]'s reach.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "the child marks itself a subreaper through the OS's C interface, prctl, between \
              fork and exec"
)]
pub(super) fn adopt_orphans(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: the closure runs in the forked child before exec, where only
    // async-signal-safe calls may run: `prctl` is a system call that sets a
    // flag on the calling process, which exec keeps, and the closure
    // allocates nothing and takes no lock.
    unsafe {
        command.pre_exec(|| {
            let on: libc::c_ulong = 1;
            if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, on) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Elsewhere a process left behind leaves the tree, as it did.
#[cfg(not(target_os = "linux"))]
pub(super) fn adopt_orphans(_command: &mut Command) {}
