//! The profile-guided release build of the static musl executables (ADR
//! 0057's note of 2026-11-14): `ci/musl/cargo` passes rustc the committed
//! profile, or the one a refresh asks for, and `ci/musl/link-cc` links an
//! instrumented build through Zig's `cc` and lays the static-PIE's sections
//! out in the committed order. Zig, cargo-zigbuild, Zig's `cc`, rustc and
//! llvm-objcopy are stand-ins that record what they were asked. The build
//! is repository tooling, so the module declares no Feature.
#![cfg(unix)]

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::workspace::{Workspace, finding};

const TARGET: &str = "x86_64-unknown-linux-musl";
const PROFILE: &[u8] = b"an indexed profile";
const ORDER: &str = "_RNvCs1_4main3hot\n.rodata..Lanon.0a1b.3\n.rodata._RNvCs1_4main3hot\n.data.rel.ro..Lanon.0a1b.4\n.rodata..Lanon.ffff.9\n_RNvCs1_4main4cold\n";

fn repository_script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ci/musl")
        .join(name)
}

fn executable(path: &Path, script: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A stand-in that writes each argument on a line of `<folder>/<name>.args`,
/// and the lines of each `@file` argument after it, indented, then runs
/// `then`.
fn recorder(folder: &Path, name: &str, then: &str) -> PathBuf {
    let path = folder.join(name);
    let record = folder.join(format!("{name}.args"));
    executable(
        &path,
        &format!(
            "#!/usr/bin/env bash\n\
             for a in \"$@\"; do\n\
               echo \"$a\" >>\"{record}\"\n\
               if [[ \"$a\" == @* ]]; then sed 's/^/  /' \"${{a#@}}\" >>\"{record}\"; fi\n\
             done\n\
             {then}\n",
            record = record.display(),
        ),
    );
    path
}

fn recorded(folder: &Path, name: &str) -> Vec<String> {
    std::fs::read_to_string(folder.join(format!("{name}.args")))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// `ci/musl/link-cc` with stand-ins for Zig's `cc`, which prints the LLD
/// command of a PIE link over `main.rcgu.o`, Zig, llvm-objcopy and
/// llvm-objdump, which lists two anonymous constants' sections.
struct Linker {
    workspace: Workspace,
    stand_in: PathBuf,
}

impl Linker {
    fn new() -> Self {
        let workspace = Workspace::from_fixture("empty");
        let stand_in = workspace.path().join("stand-in");
        let build = workspace.path().join("build");
        workspace.write("build/main.rcgu.o", b"rust code");
        recorder(
            &stand_in,
            "cc",
            &format!(
                "for a in \"$@\"; do [[ \"$a\" == -pie ]] && echo \"ld.lld -pie /zig/rcrt1.o {build}/main.rcgu.o /zig/libc.a -o {build}/main-0f1e\" >&2; done; exit 0",
                build = build.display()
            ),
        );
        recorder(&stand_in, "zig", "exit 0");
        recorder(&stand_in, "llvm-objcopy", "cp \"${@: -2:1}\" \"${@: -1}\"");
        recorder(
            &stand_in,
            "llvm-objdump",
            "printf '%s\\n' '' 'main.rcgu.o:\tfile format elf64-x86-64' '' 'Sections:' \
             'Idx Name          Size     VMA              Type' \
             '  0               00000000 0000000000000000 ' \
             '  1 .text._RNvCs1_4main3hot 00000010 0000000000000000 TEXT' \
             '  2 .rodata..Lanon.0a1b.3 00000008 0000000000000000 DATA' \
             '  3 .data.rel.ro..Lanon.0a1b.4 00000010 0000000000000000 DATA' \
             '  4 .rodata..Lanon.0a1b.5 00000008 0000000000000000 DATA' \
             '  5 .rodata._RNvCs1_4main3hot 00000020 0000000000000000 DATA'",
        );
        Self {
            workspace,
            stand_in,
        }
    }

    fn link(&self, arguments: &[&str], variables: &[(&str, &str)]) {
        let output = self
            .workspace
            .command(repository_script("link-cc"))
            .args(arguments)
            .env("PATH", "/usr/bin:/bin")
            .env("GOODFETCH_MUSL_LINK_TARGET", TARGET)
            .env("CC_x86_64_unknown_linux_musl", self.stand_in.join("cc"))
            .env("CARGO_ZIGBUILD_ZIG_PATH", self.stand_in.join("zig"))
            .env("GOODFETCH_MUSL_LLVM_TOOLS", &self.stand_in)
            .envs(variables.iter().copied())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn asked(&self, name: &str) -> Vec<String> {
        recorded(&self.stand_in, name)
    }

    fn order_file(&self, contents: &str) -> String {
        self.workspace.write("order.txt", contents.as_bytes());
        self.workspace
            .path()
            .join("order.txt")
            .display()
            .to_string()
    }

    fn build(&self) -> PathBuf {
        self.workspace.path().join("build")
    }
}

/// rustc asks an instrumented build's linker to keep the profile runtime
/// with `-u __llvm_profile_runtime`, two arguments a C compiler's driver
/// reads as an option and a file; Zig's `cc` takes the symbol for an input
/// it cannot read, so link-cc hands it to the linker in one argument.
#[test]
fn an_undefined_symbol_reaches_zigs_cc_as_a_linker_option() {
    let linker = Linker::new();

    linker.link(
        &["-u", "__llvm_profile_runtime", "main.o", "-o", "main"],
        &[],
    );

    assert_eq!(
        linker.asked("cc"),
        ["-Wl,-u,__llvm_profile_runtime", "main.o", "-o", "main"]
    );
}

/// A static-PIE's link passes its arguments in a response file when they are
/// too long for a command; the pair is joined there too, in a copy.
#[test]
fn an_undefined_symbol_in_a_response_file_reaches_zigs_cc_as_a_linker_option() {
    let linker = Linker::new();
    let response = linker.workspace.path().join("linker-arguments");
    std::fs::write(
        &response,
        "-static-pie\n-u\n__llvm_profile_runtime\nmain.o\n",
    )
    .unwrap();

    linker.link(&[&format!("@{}", response.display())], &[]);

    let asked = linker.asked("cc");
    assert_eq!(
        asked[1..],
        [
            "  -static-pie",
            "  -Wl,-u,__llvm_profile_runtime",
            "  main.o",
            "-pie"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(&response).unwrap(),
        "-static-pie\n-u\n__llvm_profile_runtime\nmain.o\n",
        "rustc's own response file is left as it wrote it"
    );
}

/// With an order file, the static-PIE's LLD lays its sections out in the
/// file's order, and writes the map that places them beside the
/// executable. An anonymous constant (`.rodata..L*`, `.data.rel.ro..L*`)
/// and a function's jump table (`.rodata.<function>`) have no symbol to
/// name them by, so the Rust object is linked from a copy
/// in which llvm-objcopy gives each the file lists and the object holds a
/// hidden symbol named like its section, which the executable keeps local.
/// (llvm-objcopy writes an added local symbol after the global ones, which
/// LLD refuses.)
#[test]
fn an_order_file_orders_the_static_pies_sections() {
    let linker = Linker::new();
    let order = linker.order_file(ORDER);

    linker.link(
        &["-static-pie", "main.rcgu.o"],
        &[("GOODFETCH_MUSL_ORDER_FILE", &order)],
    );

    let objcopy = linker.asked("llvm-objcopy");
    assert_eq!(
        objcopy[1..7],
        [
            "  --add-symbol",
            "  .rodata..Lanon.0a1b.3=.rodata..Lanon.0a1b.3:0,global,hidden,object",
            "  --add-symbol",
            "  .data.rel.ro..Lanon.0a1b.4=.data.rel.ro..Lanon.0a1b.4:0,global,hidden,object",
            "  --add-symbol",
            "  .rodata._RNvCs1_4main3hot=.rodata._RNvCs1_4main3hot:0,global,hidden,object",
        ]
    );
    assert_eq!(
        objcopy[7],
        linker.build().join("main.rcgu.o").display().to_string()
    );
    let copy = &objcopy[8];
    let lld = linker.asked("zig");
    assert_eq!(
        lld,
        [
            "ld.lld",
            "-pie",
            "/zig/rcrt1.o",
            copy,
            "/zig/libc.a",
            "-o",
            &linker.build().join("main-0f1e").display().to_string(),
            &format!("--symbol-ordering-file={order}"),
            "--no-warn-symbol-ordering",
            &format!("-Map={}", linker.build().join("main-0f1e.map").display()),
            "--no-demangle",
            "-z",
            "pack-relative-relocs",
        ]
    );
}

/// An order file of functions and named data alone needs no copy.
#[test]
fn an_order_file_without_anonymous_constants_links_the_object_itself() {
    let linker = Linker::new();
    let order = linker.order_file("_RNvCs1_4main3hot\n_RNvCs1_4main4cold\n");

    linker.link(
        &["-static-pie", "main.rcgu.o"],
        &[("GOODFETCH_MUSL_ORDER_FILE", &order)],
    );

    assert_eq!(linker.asked("llvm-objcopy"), Vec::<String>::new());
    assert_eq!(
        linker.asked("zig")[3],
        linker.build().join("main.rcgu.o").display().to_string()
    );
}

/// `ci/musl/cargo` with stand-ins for cargo-zigbuild, which records the
/// flags rustc would get, Zig and rustc, and with a committed profile and
/// order file for `x86_64-unknown-linux-musl`.
struct Build {
    workspace: Workspace,
    stand_in: PathBuf,
}

impl Build {
    fn new() -> Self {
        let workspace = Workspace::from_fixture("empty");
        for script in ["cargo", "link-cc", "toolchain.sh"] {
            let path = workspace.path().join("ci/musl").join(script);
            executable(
                &path,
                &std::fs::read_to_string(repository_script(script)).unwrap(),
            );
        }
        workspace.write("ci/pgo/x86_64-unknown-linux-musl/merged.profdata", PROFILE);
        workspace.write(
            "ci/pgo/x86_64-unknown-linux-musl/order.txt",
            ORDER.as_bytes(),
        );
        let stand_in = workspace.path().join("stand-in");
        let toolchain = Self::toolchain(&workspace);
        recorder(
            &toolchain.join("bin"),
            "cargo-zigbuild",
            &format!(
                "for name in RUSTFLAGS CARGO_ENCODED_RUSTFLAGS GOODFETCH_MUSL_ORDER_FILE GOODFETCH_MUSL_LLVM_TOOLS; do\n\
                   if [[ -n \"${{!name+set}}\" ]]; then printf '%s=%s\\n' \"$name\" \"${{!name}}\"; fi\n\
                 done >\"{}\"",
                stand_in.join("flags").display()
            ),
        );
        recorder(&toolchain.join("zig"), "zig", "exit 0");
        let sysroot = stand_in.join("sysroot");
        recorder(
            &stand_in.join("bin"),
            "rustc",
            &format!(
                "case \"$*\" in\n\
                   '--print sysroot') echo {} ;;\n\
                   '--print host-tuple') echo x86_64-unknown-linux-gnu ;;\n\
                   *) exit 1 ;;\n\
                 esac",
                sysroot.display()
            ),
        );
        recorder(
            &sysroot.join("lib/rustlib/x86_64-unknown-linux-gnu/bin"),
            "llvm-objcopy",
            "exit 0",
        );
        Self {
            workspace,
            stand_in,
        }
    }

    fn toolchain(workspace: &Workspace) -> PathBuf {
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg("source ci/musl/toolchain.sh && printf %s \"$musl_toolchain\"")
            .current_dir(workspace.path())
            .env("XDG_DATA_HOME", workspace.path().join("data"))
            .output()
            .unwrap();
        PathBuf::from(String::from_utf8(output.stdout).unwrap())
    }

    fn cache(&self) -> PathBuf {
        self.workspace.path().join("cache")
    }

    fn run(&self, arguments: &[&str], variables: &[(&str, &str)]) -> std::process::Output {
        self.workspace
            .command(self.workspace.path().join("ci/musl/cargo"))
            .args(arguments)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.stand_in.join("bin").display()),
            )
            .env("XDG_DATA_HOME", self.workspace.path().join("data"))
            .env("XDG_CACHE_HOME", self.cache())
            .envs(variables.iter().copied())
            .output()
            .unwrap()
    }

    /// The variables cargo-zigbuild saw, `NAME=value`, of those it records.
    fn flags(&self, arguments: &[&str], variables: &[(&str, &str)]) -> Vec<String> {
        let output = self.run(arguments, variables);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::read_to_string(self.stand_in.join("flags"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The one folder the committed files were copied to.
    fn copied(&self) -> PathBuf {
        let folder = self.cache().join("goodfetch/pgo");
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 1, "{entries:?}");
        entries.remove(0)
    }

    fn llvm_tools(&self) -> String {
        format!(
            "GOODFETCH_MUSL_LLVM_TOOLS={}",
            self.stand_in
                .join("sysroot/lib/rustlib/x86_64-unknown-linux-gnu/bin")
                .display()
        )
    }
}

fn profile_use(folder: &Path) -> String {
    format!(
        "-Cprofile-use={} -Cllvm-args=-no-pgo-warn-mismatch",
        folder.join("merged.profdata").display()
    )
}

/// A release build for the musl target the files are for compiles with
/// the profile and links in the order, from a copy named by their contents:
/// a refreshed profile or order file changes the flags Cargo fingerprints,
/// so it rebuilds, and checkouts with the same files share compiled code.
#[rstest]
#[case::release(&["--release"])]
#[case::release_short(&["-r"])]
#[case::release_profile(&["--profile", "release"])]
#[case::release_profile_joined(&["--profile=release"])]
fn a_release_musl_build_uses_the_committed_profile(#[case] profile: &[&str]) {
    let build = Build::new();
    let mut arguments = vec!["build", "--target", TARGET];
    arguments.extend_from_slice(profile);

    let flags = build.flags(&arguments, &[]);

    let copied = build.copied();
    assert_eq!(
        flags,
        [
            format!("RUSTFLAGS={}", profile_use(&copied)),
            format!(
                "GOODFETCH_MUSL_ORDER_FILE={}",
                copied.join("order.txt").display()
            ),
            build.llvm_tools(),
        ]
    );
    assert_eq!(
        std::fs::read(copied.join("merged.profdata")).unwrap(),
        PROFILE
    );
    assert_eq!(
        std::fs::read_to_string(copied.join("order.txt")).unwrap(),
        ORDER
    );
}

#[test]
fn a_refreshed_order_file_changes_the_flags() {
    let build = Build::new();
    let arguments = ["build", "--release", "--target", TARGET];
    let before = build.flags(&arguments, &[]);
    build.workspace.append(
        "ci/pgo/x86_64-unknown-linux-musl/order.txt",
        "_RNvCs1_4main4warm",
    );

    let after = build.flags(&arguments, &[]);

    assert_ne!(before[0], after[0]);
}

/// Every other build compiles as before: another profile (`ci` inherits
/// the release profile but is no release), another target, a musl target
/// without files, or a build that turns the profile off.
#[rstest]
#[case::dev_profile(&["build", "--target", TARGET], &[])]
#[case::ci_profile(&["build", "--profile", "ci", "--target", TARGET], &[])]
#[case::gnu_target(&["build", "--release", "--target", "x86_64-unknown-linux-gnu"], &[])]
#[case::target_without_files(&["build", "--release", "--target", "aarch64-unknown-linux-musl"], &[])]
#[case::turned_off(&["build", "--release", "--target", TARGET], &[("GOODFETCH_PGO", "off")])]
fn other_builds_compile_without_a_profile(
    #[case] arguments: &[&str],
    #[case] variables: &[(&str, &str)],
) {
    let build = Build::new();

    let flags = build.flags(arguments, variables);

    assert_eq!(flags, Vec::<String>::new());
}

/// The flags the caller gives rustc are kept, in the variable Cargo reads.
#[rstest]
#[case::rustflags("RUSTFLAGS", "--cfg loom", " ")]
#[case::encoded("CARGO_ENCODED_RUSTFLAGS", "--cfg\u{1f}loom", "\u{1f}")]
fn the_callers_flags_are_kept(
    #[case] variable: &str,
    #[case] value: &str,
    #[case] separator: &str,
) {
    let build = Build::new();

    let flags = build.flags(
        &["build", "--release", "--target", TARGET],
        &[(variable, value)],
    );

    let copied = build.copied();
    assert_eq!(
        flags[0],
        format!(
            "{variable}={value}{separator}{}",
            profile_use(&copied).replace(' ', separator)
        )
    );
}

/// A refresh first builds the executables instrumented, whatever the
/// profile, with neither profile nor order.
#[test]
fn an_instrumented_build_writes_its_profiles_where_asked() {
    let build = Build::new();

    let flags = build.flags(
        &["build", "--release", "--target", TARGET],
        &[("GOODFETCH_PGO", "generate=/raw")],
    );

    assert_eq!(flags, ["RUSTFLAGS=-Cprofile-generate=/raw"]);
}

/// Then it builds with the profile it merged, which it reads the hot order
/// from, before any order file exists.
#[rstest]
#[case::with_an_order(true)]
#[case::profile_alone(false)]
fn a_build_uses_the_profile_in_a_folder_asked_for(#[case] with_order: bool) {
    let build = Build::new();
    build
        .workspace
        .write("merged/merged.profdata", b"a new profile");
    if with_order {
        build
            .workspace
            .write("merged/order.txt", b"_RNvCs1_4main3hot\n");
    }
    let folder = build.workspace.path().join("merged");

    let flags = build.flags(
        &["build", "--release", "--target", TARGET],
        &[("GOODFETCH_PGO", &format!("use={}", folder.display()))],
    );

    let copied = build.copied();
    assert_eq!(
        std::fs::read(copied.join("merged.profdata")).unwrap(),
        b"a new profile"
    );
    let mut expected = vec![format!("RUSTFLAGS={}", profile_use(&copied))];
    if with_order {
        expected.push(format!(
            "GOODFETCH_MUSL_ORDER_FILE={}",
            copied.join("order.txt").display()
        ));
        expected.push(build.llvm_tools());
    }
    assert_eq!(flags, expected);
}

#[test]
fn an_unknown_setting_fails() {
    let build = Build::new();

    let output = build.run(
        &["build", "--release", "--target", TARGET],
        &[("GOODFETCH_PGO", "on")],
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("GOODFETCH_PGO"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An ordered build without the LLVM tools that order it stops before
/// building, naming the recipe that installs them.
#[test]
fn an_ordered_build_without_the_llvm_tools_fails() {
    let build = Build::new();
    std::fs::remove_file(
        build
            .stand_in
            .join("sysroot/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-objcopy"),
    )
    .unwrap();

    let output = build.run(&["build", "--release", "--target", TARGET], &[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("just musl-toolchain"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!build.stand_in.join("flags").exists());
}

/// A release build of both Products for `xtask pgo check`: each Product's
/// path a hard link to the file the linker wrote in `deps/`, named after
/// the crate, as Cargo leaves it, with the link map ci/musl/link-cc wrote beside that file.
struct Built {
    workspace: Workspace,
}

impl Built {
    /// A build whose target has `order` as its committed order file, or
    /// none.
    fn new(order: Option<&str>) -> Self {
        let workspace = Workspace::from_fixture("empty");
        if let Some(order) = order {
            workspace.write(&format!("ci/pgo/{TARGET}/order.txt"), order.as_bytes());
        }
        let release = workspace.path().join("build/release");
        workspace.write(
            "tools.json",
            serde_json::json!({
                "collie": release.join("goodfetch-collie"),
                "foxhound": release.join("goodfetch-foxhound"),
            })
            .to_string()
            .as_bytes(),
        );
        Self { workspace }
    }

    /// Links `binary`, its `.text` holding `functions` at these addresses,
    /// with its link map unless `map` is false.
    fn link(&self, binary: &str, functions: &[(u64, &str)], map: bool) {
        // Cargo names the file in `deps/` after the crate: `-` becomes `_`.
        let crate_name = binary.replace('-', "_");
        let deps = format!("build/release/deps/{crate_name}-0f1e");
        self.workspace.write(&deps, b"an executable");
        // An unrelated build's file of the same Product, which is no link.
        self.workspace
            .write(&format!("build/release/deps/{crate_name}-9a8b"), b"older");
        std::fs::hard_link(
            self.workspace.path().join(&deps),
            self.workspace
                .path()
                .join(format!("build/release/{binary}")),
        )
        .unwrap();
        if map {
            let mut text = String::from(
                "             VMA              LMA     Size Align Out     In      Symbol\n",
            );
            writeln!(
                text,
                "{0:>16x} {0:>16x} {1:>8x} {2:>5} .text",
                0x1000, 0x1000, 16
            )
            .unwrap();
            for &(address, name) in functions {
                writeln!(
                    text,
                    "{address:>16x} {address:>16x} {:>8x} {:>5}         /b/main.rcgu.o:(.text.{name})",
                    0x10, 16
                )
                .unwrap();
                writeln!(
                    text,
                    "{address:>16x} {address:>16x} {:>8x} {:>5}                 {name}",
                    0x10, 1
                )
                .unwrap();
            }
            self.workspace
                .write(&format!("{deps}.map"), text.as_bytes());
        }
    }

    fn check(&self) -> crate::workspace::Outcome {
        self.workspace
            .xtask(&["pgo", "check", "--tools", "tools.json", "--target", TARGET])
    }

    fn subject(&self, binary: &str) -> String {
        self.workspace
            .path()
            .join("build/release")
            .join(binary)
            .display()
            .to_string()
    }
}

const ORDERED: &str = "hot\nwarm\ntepid\n.rodata..Lanon.0a1b.3\n";

#[test]
fn a_build_whose_text_starts_in_the_order_files_order_passes() {
    let built = Built::new(Some(ORDERED));
    built.link(
        "goodfetch-collie",
        &[(0x1000, "hot"), (0x1010, "warm"), (0x1020, "cold")],
        true,
    );
    built.link(
        "goodfetch-foxhound",
        &[(0x1000, "hot"), (0x1010, "warm")],
        true,
    );

    let outcome = built.check();

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), []);
}

#[test]
fn a_build_out_of_the_order_files_order_fails() {
    let built = Built::new(Some(ORDERED));
    built.link(
        "goodfetch-collie",
        &[(0x1000, "hot"), (0x1010, "warm")],
        true,
    );
    // `tepid` placed before `warm`, which the file lists ahead of it.
    built.link(
        "goodfetch-foxhound",
        &[(0x1000, "hot"), (0x1010, "tepid"), (0x1020, "warm")],
        true,
    );

    let outcome = built.check();

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [finding(
            "out-of-order",
            &built.subject("goodfetch-foxhound")
        )]
    );
}

#[test]
fn a_build_without_a_link_map_fails() {
    let built = Built::new(Some(ORDERED));
    built.link(
        "goodfetch-collie",
        &[(0x1000, "hot"), (0x1010, "warm")],
        false,
    );
    built.link(
        "goodfetch-foxhound",
        &[(0x1000, "hot"), (0x1010, "warm")],
        true,
    );

    let outcome = built.check();

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [finding("no-link-map", &built.subject("goodfetch-collie"))]
    );
}

#[test]
fn a_target_without_an_order_file_is_noted_unordered() {
    let built = Built::new(None);

    let outcome = built.check();

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), []);
    assert_eq!(outcome.notes(), [finding("not-ordered", TARGET)]);
}
