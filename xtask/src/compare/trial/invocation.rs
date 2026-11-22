//! How a cell's tool is started, and what it takes on disk: its
//! executable, or the interpreter that runs its side's script, with the
//! arguments that come before the trial's own.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::footprint;
use crate::compare::matrix::Config;

/// The program that runs a cell's tool and the arguments that come before
/// the trial's own: the executable itself, or for a tool run by an
/// interpreter, the interpreter with the tool's script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub(crate) program: PathBuf,
    pub(crate) arguments: Vec<OsString>,
}

/// Node.js's executable, which runs Crawlee's side and Foxhound's
/// program.
const NODE: &str = "node";

/// The Python environment `uv sync` makes beside Scrapy's side, and its
/// interpreter in it.
const PYTHON_ENVIRONMENT: &str = ".venv";
#[cfg(not(windows))]
const PYTHON: &str = "bin/python";
#[cfg(windows)]
const PYTHON: &str = "Scripts/python.exe";

/// The packages `pnpm install` leaves beside Crawlee's side and
/// Foxhound's program.
const NODE_MODULES: &str = "node_modules";

/// The first file named `name` in a folder of `PATH`.
fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|folder| folder.join(name))
        .find(|path| path.is_file())
}

/// The folder of a side's script.
fn side_of(script: &Path) -> &Path {
    script.parent().unwrap_or_else(|| Path::new("."))
}

/// fastCRW's server, which its side starts: `just compare` installs it
/// into the `bin` folder beside the side's executable.
pub(crate) fn fastcrw_server(executable: &Path) -> PathBuf {
    side_of(executable)
        .join("bin")
        .join(format!("crw-server{}", std::env::consts::EXE_SUFFIX))
}

/// How `config`'s tool runs `executable`: Foxhound's, Spider's,
/// crawlee-rs's, pagesift's, OpenSpider's, fastCRW's and crawlberg's are
/// executables, Spider's side given its row's page channel capacity and
/// fastCRW's the server it starts, Crawlee's side's script and Foxhound's
/// program are run by Node.js from `PATH`, and Scrapy's its side's script run by the
/// interpreter of the Python environment beside it.
pub(crate) fn invocation(config: Config, executable: &Path) -> Invocation {
    match config {
        Config::Foxhound { .. }
        | Config::CrawleeRs
        | Config::Pagesift
        | Config::OpenSpider
        | Config::Crawlberg => Invocation {
            program: executable.to_owned(),
            arguments: Vec::new(),
        },
        Config::FastCrw => Invocation {
            program: executable.to_owned(),
            arguments: vec!["--server".into(), fastcrw_server(executable).into()],
        },
        Config::Spider { capacity, .. } => Invocation {
            program: executable.to_owned(),
            arguments: vec!["--capacity".into(), capacity.to_string().into()],
        },
        Config::Crawlee | Config::FoxhoundProgram => Invocation {
            program: on_path(NODE).unwrap_or_else(|| PathBuf::from(NODE)),
            arguments: vec![executable.as_os_str().to_owned()],
        },
        Config::Scrapy => Invocation {
            program: side_of(executable).join(PYTHON_ENVIRONMENT).join(PYTHON),
            arguments: vec![executable.as_os_str().to_owned()],
        },
    }
}

/// What a tool takes on disk: its executable (and fastCRW's server beside
/// it), and for a tool run by an interpreter, the interpreter's executable
/// and the packages its side installed (`node_modules`, or the Python
/// environment; the Python standard library outside it is not counted);
/// with `engine`, the Foxhound executable its program's SDK starts; with
/// `browser`, the browser's folder too, the same for every tool.
pub(super) fn install_bytes(
    config: Config,
    executable: &Path,
    engine: Option<&Path>,
    browser: Option<&Path>,
) -> u64 {
    let file_bytes = |path: &Path| {
        std::fs::canonicalize(path)
            .and_then(std::fs::metadata)
            .map_or(0, |metadata| metadata.len())
    };
    let browser_bytes = browser.map_or(0, footprint);
    let packages = match config {
        Config::Foxhound { .. }
        | Config::Spider { .. }
        | Config::CrawleeRs
        | Config::Pagesift
        | Config::OpenSpider
        | Config::Crawlberg => {
            return file_bytes(executable) + browser_bytes;
        }
        Config::FastCrw => {
            return file_bytes(executable)
                + file_bytes(&fastcrw_server(executable))
                + browser_bytes;
        }
        Config::Crawlee | Config::FoxhoundProgram => NODE_MODULES,
        Config::Scrapy => PYTHON_ENVIRONMENT,
    };
    file_bytes(&invocation(config, executable).program)
        + footprint(&side_of(executable).join(packages))
        + engine.map_or(0, file_bytes)
        + browser_bytes
}
