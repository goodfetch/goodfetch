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
/// The rustc a profile-guided build compiles through, in `ci/musl/`.
const STABLE_RUSTC: &str = "stable-metadata/rustc";
/// The flag that marks a profile-guided build's own crates for it.
const STABLE_MARK: &str = "-Cmetadata=goodfetch-stable-metadata";
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

/// Replaces the file at `path` with its zstd copy, `<path>.zst`, as the
/// files are committed.
fn compress(path: &Path) {
    let status = std::process::Command::new("zstd")
        .args(["-q", "--rm"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
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
        for script in ["cargo", "link-cc", "toolchain.sh", STABLE_RUSTC] {
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
                "for name in RUSTFLAGS CARGO_ENCODED_RUSTFLAGS GOODFETCH_MUSL_ORDER_FILE GOODFETCH_MUSL_LLVM_TOOLS RUSTC GOODFETCH_PGO_RUSTC; do\n\
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

    /// The variables of a build through ci/musl/stable-metadata/rustc,
    /// which runs the caller's rustc, the one on `PATH` here.
    fn stable_rustc(&self) -> [String; 2] {
        [
            format!(
                "RUSTC={}",
                self.workspace
                    .path()
                    .join("ci/musl")
                    .join(STABLE_RUSTC)
                    .display()
            ),
            "GOODFETCH_PGO_RUSTC=rustc".to_owned(),
        ]
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
        "{STABLE_MARK} -Cprofile-use={} -Cllvm-args=-no-pgo-warn-mismatch",
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
            build.stable_rustc()[0].clone(),
            build.stable_rustc()[1].clone(),
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
fn a_compressed_profile_and_order_file_are_used_as_committed() {
    let build = Build::new();
    let committed = build.workspace.path().join("ci/pgo").join(TARGET);
    compress(&committed.join("merged.profdata"));
    compress(&committed.join("order.txt"));

    build.flags(&["build", "--release", "--target", TARGET], &[]);

    let copied = build.copied();
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

    let mut expected = vec![format!("RUSTFLAGS={STABLE_MARK} -Cprofile-generate=/raw")];
    expected.extend(build.stable_rustc());
    assert_eq!(flags, expected);
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
    expected.extend(build.stable_rustc());
    assert_eq!(flags, expected);
}

/// Every profile-guided build, instrumented or guided, compiles through
/// ci/musl/stable-metadata/rustc, the caller's rustc after it, and marks
/// its crates' flags for it: a crate's symbol names then hold across
/// Cargo.toml and Cargo.lock changes, so the profile's records still name
/// the crates they were taken from.
#[rstest]
#[case::committed(None)]
#[case::instrumented(Some("generate=/raw"))]
#[case::asked_for(Some("use=merged"))]
fn a_profile_guided_build_compiles_through_the_stable_metadata_rustc(
    #[case] setting: Option<&str>,
) {
    let build = Build::new();
    build
        .workspace
        .write("merged/merged.profdata", b"a new profile");
    let setting = setting.map(|setting| {
        setting.replace(
            "merged",
            &build.workspace.path().join("merged").display().to_string(),
        )
    });
    let mut variables = vec![("RUSTC", "/opt/rust/bin/rustc")];
    if let Some(setting) = &setting {
        variables.push(("GOODFETCH_PGO", setting));
    }

    let flags = build.flags(&["build", "--release", "--target", TARGET], &variables);

    let rustflags = flags
        .iter()
        .find_map(|flag| flag.strip_prefix("RUSTFLAGS="))
        .unwrap();
    assert!(
        rustflags.split(' ').any(|flag| flag == STABLE_MARK),
        "{rustflags}"
    );
    assert!(
        flags.contains(&format!(
            "RUSTC={}",
            build
                .workspace
                .path()
                .join("ci/musl")
                .join(STABLE_RUSTC)
                .display()
        )),
        "{flags:?}"
    );
    assert!(
        flags.contains(&"GOODFETCH_PGO_RUSTC=/opt/rust/bin/rustc".to_owned()),
        "{flags:?}"
    );
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

/// The order file is committed compressed; the plain file is read too.
#[rstest]
#[case::compressed(true)]
#[case::plain(false)]
fn a_build_out_of_the_order_files_order_fails(#[case] compressed: bool) {
    let built = Built::new(Some(ORDERED));
    if compressed {
        compress(
            &built
                .workspace
                .path()
                .join(format!("ci/pgo/{TARGET}/order.txt")),
        );
    }
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

/// ci/musl/stable-metadata/rustc compiling crates with the real rustc, as
/// Cargo asks it to: each crate's package and version in the variables
/// Cargo sets, and Cargo's own `-C metadata`.
struct Compiler {
    workspace: Workspace,
}

/// A library crate with a function, a closure and a generic instance.
const LIBRARY: &str = "pub fn kept(n: u64) -> u64 { (0..n).filter(|i| i % 3 == 0).sum() }\n\
                       pub fn boxed(n: u64) -> Vec<u64> { vec![n] }\n";

/// A crate as Cargo describes it to rustc.
struct Unit<'a> {
    name: &'a str,
    version: &'a str,
    /// Cargo's `-C metadata` for it.
    metadata: &'a str,
    /// Whether ci/musl/cargo's marker is among its flags: Cargo passes them
    /// to the target's crates alone.
    marked: bool,
}

impl Compiler {
    fn new() -> Self {
        Self {
            workspace: Workspace::from_fixture("empty"),
        }
    }

    /// Compiles `source` as `unit` with `arguments` added, in the folder
    /// `out`.
    fn compile(
        &self,
        unit: &Unit<'_>,
        source: &str,
        out: &str,
        arguments: &[&str],
    ) -> std::process::Output {
        let folder = self.workspace.path().join(out);
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("source.rs");
        std::fs::write(&file, source).unwrap();
        let (version, pre) = unit.version.split_once('-').unwrap_or((unit.version, ""));
        let parts: Vec<&str> = version.split('.').collect();
        let mut command = std::process::Command::new(repository_script(STABLE_RUSTC));
        command
            .current_dir(&folder)
            .env("GOODFETCH_PGO_RUSTC", "rustc")
            .env("CARGO_PKG_NAME", unit.name)
            .env("CARGO_PKG_VERSION", unit.version)
            .env("CARGO_PKG_VERSION_MAJOR", parts[0])
            .env("CARGO_PKG_VERSION_MINOR", parts[1])
            .env("CARGO_PKG_VERSION_PATCH", parts[2])
            .env("CARGO_PKG_VERSION_PRE", pre)
            .args(["--edition", "2024", "--crate-name", unit.name])
            .args(["-C", &format!("metadata={}", unit.metadata)])
            .args(["-C", "symbol-mangling-version=v0"]);
        if unit.marked {
            command.arg(STABLE_MARK);
        }
        command.args(arguments).arg(&file);
        command.output().unwrap()
    }

    /// The module name and the functions LLVM gets for `unit`'s library.
    fn names(&self, unit: &Unit<'_>, out: &str) -> Vec<String> {
        let output = self.compile(
            unit,
            LIBRARY,
            out,
            &[
                "--crate-type",
                "lib",
                "--emit",
                "llvm-ir",
                "-o",
                "library.ll",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = self.workspace.read_text(&format!("{out}/library.ll"));
        text.lines()
            .filter(|line| line.starts_with("; ModuleID") || line.starts_with("define "))
            .map(|line| {
                line.split_once('@')
                    .map_or(line, |(_, name)| name.split('(').next().unwrap())
                    .to_owned()
            })
            .collect()
    }
}

/// A marked crate's names are its package's and version's own: what Cargo
/// derives its metadata from (the dependencies' versions, the features,
/// the lock file) changes them no more, nor does a release Cargo treats as
/// compatible, so a profile's records still name the crate.
#[rstest]
#[case::other_cargo_metadata("1.2.3", "1.2.3")]
#[case::a_patch_release("1.2.3", "1.2.9")]
#[case::a_minor_release("1.2.3", "1.4.0")]
#[case::a_patch_release_before_1("0.4.1", "0.4.7")]
fn a_crates_names_hold_across_cargos_metadata(#[case] before: &str, #[case] after: &str) {
    let compiler = Compiler::new();
    let unit = |version, metadata| Unit {
        name: "probe",
        version,
        metadata,
        marked: true,
    };

    let first = compiler.names(&unit(before, "0f1e2d3c4b5a6978"), "before");
    let second = compiler.names(&unit(after, "8796a5b4c3d2e1f0"), "after");

    assert_eq!(first, second);
}

/// Releases Cargo keeps apart in one build get names apart, and so does a
/// crate Cargo builds for the host (a build script, a procedural macro),
/// whose flags hold no marker: its metadata stays Cargo's.
#[rstest]
#[case::a_minor_release_before_1("0.4.1", "0.5.0", true)]
#[case::a_major_release("1.2.3", "2.0.0", true)]
#[case::a_pre_release("1.0.0-alpha.1", "1.0.0-alpha.2", true)]
#[case::a_host_crate("1.2.3", "1.2.3", false)]
fn other_crates_names_differ(#[case] before: &str, #[case] after: &str, #[case] marked: bool) {
    let compiler = Compiler::new();
    let unit = |version, metadata| Unit {
        name: "probe",
        version,
        metadata,
        marked,
    };

    let first = compiler.names(&unit(before, "0f1e2d3c4b5a6978"), "before");
    let second = compiler.names(&unit(after, "8796a5b4c3d2e1f0"), "after");

    assert_ne!(first, second);
}

/// Two crates of one package and version in one build, which Cargo's
/// metadata alone told apart (two sources, or two sets of features), get
/// the same crate identity: rustc then refuses to build the crate that
/// links both, loudly, rather than mixing their profiles.
#[test]
fn two_crates_of_one_package_and_version_fail_the_build() {
    let compiler = Compiler::new();
    for (out, metadata) in [
        ("first", "0f1e2d3c4b5a6978"),
        ("second", "8796a5b4c3d2e1f0"),
    ] {
        let unit = Unit {
            name: "probe",
            version: "1.2.3",
            metadata,
            marked: true,
        };
        let output = compiler.compile(
            &unit,
            LIBRARY,
            out,
            &["--crate-type", "rlib", "--out-dir", "."],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let both = Unit {
        name: "both",
        version: "1.0.0",
        metadata: "00112233",
        marked: true,
    };
    let first = compiler.workspace.path().join("first/libprobe.rlib");
    let second = compiler.workspace.path().join("second/libprobe.rlib");

    let output = compiler.compile(
        &both,
        "pub fn both() -> u64 { first::kept(3) + second::kept(4) }\n",
        "both",
        &[
            "--crate-type",
            "rlib",
            "--extern",
            &format!("first={}", first.display()),
            "--extern",
            &format!("second={}", second.display()),
        ],
    );

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("colliding StableCrateId"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A program the training runs: `changed` is the function a later commit
/// rewrites.
const TRAINED: &str = "#[inline(never)] pub fn kept(n: u64) -> u64 { let mut s = 0; for i in 0..n { if i % 3 == 0 { s += i } } s }\n\
                       #[inline(never)] pub fn changed(n: u64) -> u64 { if n > 5 { n * 2 } else { n } }\n\
                       fn main() { let n = std::hint::black_box(100); println!(\"{}\", kept(n) + changed(n)); }\n";

/// The same program after that commit.
const REWRITTEN: &str = "#[inline(never)] pub fn kept(n: u64) -> u64 { let mut s = 0; for i in 0..n { if i % 3 == 0 { s += i } } s }\n\
                         #[inline(never)] pub fn changed(n: u64) -> u64 { if n > 5 { n * 2 } else if n > 2 { n + 1 } else { n } }\n\
                         fn main() { let n = std::hint::black_box(100); println!(\"{}\", kept(n) + changed(n)); }\n";

/// A profile trained before Cargo's metadata changed still guides the
/// functions whose code is the same, and LLVM drops the records of a
/// function whose code changed (their hash no longer matches) rather than
/// apply stale counts to it. The LLVM tools are rustup's `llvm-tools`,
/// which `just musl-toolchain` adds, as `xtask pgo refresh` needs.
#[cfg(target_os = "linux")]
#[test]
fn a_profile_guides_the_functions_whose_code_held_and_drops_the_rest() {
    let compiler = Compiler::new();
    let unit = |metadata| Unit {
        name: "probe",
        version: "1.2.3",
        metadata,
        marked: true,
    };
    let raw = compiler.workspace.path().join("raw");
    let output = compiler.compile(
        &unit("0f1e2d3c4b5a6978"),
        TRAINED,
        "instrumented",
        &[
            "-O",
            "-C",
            &format!("profile-generate={}", raw.display()),
            "-o",
            "probe",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ran = std::process::Command::new(compiler.workspace.path().join("instrumented/probe"))
        .status()
        .unwrap();
    assert!(ran.success());
    let sysroot = rustc_print("sysroot");
    let profdata = std::path::Path::new(&sysroot)
        .join("lib/rustlib")
        .join(rustc_print("host-tuple"))
        .join("bin/llvm-profdata");
    assert!(
        profdata.is_file(),
        "{} is missing: run `just musl-toolchain`",
        profdata.display()
    );
    let merged = compiler.workspace.path().join("merged.profdata");
    let merge = std::process::Command::new(&profdata)
        .arg("merge")
        .arg("-o")
        .arg(&merged)
        .arg(&raw)
        .status()
        .unwrap();
    assert!(merge.success());

    let output = compiler.compile(
        &unit("8796a5b4c3d2e1f0"),
        REWRITTEN,
        "guided",
        &[
            "-O",
            "-C",
            &format!("profile-use={}", merged.display()),
            "--emit",
            "llvm-ir",
            "-o",
            "probe.ll",
        ],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ir = compiler.workspace.read_text("guided/probe.ll");
    let definition = |function: &str| {
        ir.lines()
            .find(|line| {
                line.starts_with("define ")
                    && line.contains(&format!("5probe{}{function}(", function.len()))
            })
            .unwrap_or_else(|| panic!("no definition of {function}"))
            .to_owned()
    };
    assert!(
        definition("kept").contains("!prof"),
        "{}",
        definition("kept")
    );
    assert!(
        !definition("changed").contains("!prof"),
        "{}",
        definition("changed")
    );
}

/// What `rustc --print <what>` prints.
#[cfg(target_os = "linux")]
fn rustc_print(what: &str) -> String {
    let output = std::process::Command::new("rustc")
        .args(["--print", what])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
