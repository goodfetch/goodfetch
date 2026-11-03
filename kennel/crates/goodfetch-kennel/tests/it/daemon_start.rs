//! The "Start Daemon" action against an installed executables' list and a
//! stand-in Product executable, a script that records how it was run.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write a home's files on the test thread, outside any runtime"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use goodfetch_kennel::daemon_start::{self, InstalledProduct, StartFailure};
use goodfetch_kennel::home::Home;
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable, InstalledExecutables};

use crate::support::runtime;

/// Writes a stand-in executable at `path` that writes its arguments to
/// `arguments` beside it, then exits with `status`.
fn stand_in(path: &Path, status: i32) {
    let record = path.with_file_name("arguments");
    fs::write(
        path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nexit {status}\n",
            record.display()
        ),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// The list entry of the executable at `path`, at its size and modification
/// time as the engine records them, in whole seconds.
fn entry(product: Product, path: &Path) -> InstalledExecutable {
    let metadata = fs::metadata(path).unwrap();
    let modified = jiff::Timestamp::try_from(metadata.modified().unwrap()).unwrap();
    InstalledExecutable {
        kind: ExecutableKind::ProductExecutable,
        product,
        product_version: ProductVersion::parse("0.1.0").unwrap(),
        path: path.to_string_lossy().into_owned(),
        size: metadata.len(),
        modified_at: jiff::Timestamp::from_second(modified.as_second()).unwrap(),
        pins: Vec::new(),
        copy: None,
        last_seen: modified,
    }
}

fn write_list(home: &Path, executables: Vec<InstalledExecutable>) {
    let list = InstalledExecutables {
        format_version: ProtocolVersion { major: 0, minor: 1 },
        executables,
    };
    fs::write(
        home.join("installed-executables.json"),
        serde_json::to_vec(&list).unwrap(),
    )
    .unwrap();
}

struct Installed {
    _folder: tempfile::TempDir,
    home: Home,
    bin: PathBuf,
}

fn installed() -> Installed {
    let folder = tempfile::tempdir().unwrap();
    let bin = folder.path().join("bin");
    fs::create_dir(&bin).unwrap();
    Installed {
        home: Home::new(folder.path()),
        _folder: folder,
        bin,
    }
}

#[test]
fn start_daemon_runs_the_installed_products_daemon_start_by_its_absolute_path() {
    let installed = installed();
    let foxhound = installed.bin.join("goodfetch-foxhound");
    stand_in(&foxhound, 0);
    write_list(
        installed.home.path(),
        vec![entry(Product::Foxhound, &foxhound)],
    );

    let found = daemon_start::find(&installed.home).unwrap();
    let ended = runtime().block_on(daemon_start::start(&found));

    assert_eq!(
        found,
        InstalledProduct {
            product: Product::Foxhound,
            path: foxhound.clone(),
        }
    );
    assert_eq!(ended, Ok(()));
    assert_eq!(
        fs::read_to_string(installed.bin.join("arguments")).unwrap(),
        "daemon\nstart\n"
    );
}

#[test]
fn foxhound_is_preferred_since_it_contains_collie() {
    let installed = installed();
    let collie = installed.bin.join("goodfetch-collie");
    let foxhound = installed.bin.join("goodfetch-foxhound");
    stand_in(&collie, 0);
    stand_in(&foxhound, 0);
    write_list(
        installed.home.path(),
        vec![
            entry(Product::Collie, &collie),
            entry(Product::Foxhound, &foxhound),
        ],
    );

    let found = daemon_start::find(&installed.home).unwrap();

    assert_eq!(found.product, Product::Foxhound);
}

#[test]
fn an_executable_no_longer_at_its_recorded_path_is_not_offered() {
    let installed = installed();
    let foxhound = installed.bin.join("goodfetch-foxhound");
    stand_in(&foxhound, 0);
    let mut recorded = entry(Product::Foxhound, &foxhound);
    recorded.size += 1;
    write_list(installed.home.path(), vec![recorded]);

    assert_eq!(daemon_start::find(&installed.home), None);
}

#[test]
fn a_home_without_an_installed_executables_list_offers_no_start() {
    let installed = installed();

    assert_eq!(daemon_start::find(&installed.home), None);
}

#[test]
fn a_failed_daemon_start_reports_its_exit_status() {
    let installed = installed();
    let foxhound = installed.bin.join("goodfetch-foxhound");
    stand_in(&foxhound, 3);

    let ended = runtime().block_on(daemon_start::start(&InstalledProduct {
        product: Product::Foxhound,
        path: foxhound,
    }));

    assert_eq!(ended, Err(StartFailure::Exited { code: Some(3) }));
}
