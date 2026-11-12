//! `xtask static-pie`: each Product's executable built for a static musl
//! target is a static-PIE with packed relative relocations, so it loads at
//! a randomised base (ADR 0024's note of 2026-11-12). The files are ELF
//! images written here with the header fields, program headers and dynamic
//! entries the check reads, and no code.

use std::fmt::Write as _;

use rstest::rstest;

use crate::workspace::{Finding, Outcome, Workspace, finding};

const MUSL_X64: &str = "x86_64-unknown-linux-musl";
const MUSL_ARM64: &str = "aarch64-unknown-linux-musl";

/// `e_type`: an executable linked at a fixed base.
const ET_EXEC: u16 = 2;
/// `e_type`: a position-independent executable.
const ET_DYN: u16 = 3;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const DT_NEEDED: u64 = 1;
/// A static-PIE's flags entry, which the check does not read.
const DT_FLAGS_1: u64 = 0x6fff_fffb;
const DT_RELR: u64 = 36;

/// What a linked ELF executable says about how it loads.
struct Elf {
    kind: u16,
    interpreter: bool,
    /// The dynamic section's entries before its end; `None` for a file
    /// without the section.
    dynamic: Option<Vec<(u64, u64)>>,
}

impl Elf {
    /// What the static musl build links: position-independent, with no
    /// interpreter, no library and packed relocations.
    fn static_pie() -> Self {
        Self {
            kind: ET_DYN,
            interpreter: false,
            dynamic: Some(vec![(DT_FLAGS_1, 0x0800_0001), (DT_RELR, 0x2b8)]),
        }
    }

    /// A 64-bit little-endian image: the file header, the program headers,
    /// then each segment's bytes.
    fn bytes(&self) -> Vec<u8> {
        let mut segments: Vec<(u32, Vec<u8>)> = Vec::new();
        if self.interpreter {
            segments.push((PT_INTERP, b"/lib/ld-musl-x86_64.so.1\0".to_vec()));
        }
        if let Some(entries) = &self.dynamic {
            let mut dynamic = Vec::new();
            for (tag, value) in entries.iter().chain([&(0, 0)]) {
                dynamic.extend(tag.to_le_bytes());
                dynamic.extend(value.to_le_bytes());
            }
            segments.push((PT_DYNAMIC, dynamic));
        }
        let (header_size, program_header_size) = (64_u16, 56_u16);
        let count = u16::try_from(segments.len()).unwrap();

        let mut image = b"\x7fELF\x02\x01\x01".to_vec();
        image.resize(16, 0);
        image.extend(self.kind.to_le_bytes());
        image.extend(62_u16.to_le_bytes()); // e_machine
        image.extend(1_u32.to_le_bytes()); // e_version
        image.extend(0_u64.to_le_bytes()); // e_entry
        image.extend(u64::from(header_size).to_le_bytes()); // e_phoff
        image.extend(0_u64.to_le_bytes()); // e_shoff
        image.extend(0_u32.to_le_bytes()); // e_flags
        image.extend(header_size.to_le_bytes());
        image.extend(program_header_size.to_le_bytes());
        image.extend(count.to_le_bytes());
        image.extend([0; 6]); // no section headers

        let mut offset = u64::from(header_size) + u64::from(program_header_size * count);
        let mut contents = Vec::new();
        for (kind, bytes) in segments {
            let size = bytes.len() as u64;
            image.extend(kind.to_le_bytes());
            image.extend(4_u32.to_le_bytes()); // p_flags
            image.extend(offset.to_le_bytes());
            image.extend(offset.to_le_bytes()); // p_vaddr
            image.extend(offset.to_le_bytes()); // p_paddr
            image.extend(size.to_le_bytes()); // p_filesz
            image.extend(size.to_le_bytes()); // p_memsz
            image.extend(8_u64.to_le_bytes()); // p_align
            offset += size;
            contents.extend(bytes);
        }
        image.extend(contents);
        image
    }
}

/// Writes each Product's executable for `target` and the manifest naming
/// them, as a Release cell's `ci/release/size-manifest.sh` does.
fn write_executables(workspace: &Workspace, target: &str, contents: &[u8]) {
    let mut manifest = "profile = \"ci\"\n".to_owned();
    for product in ["collie", "foxhound"] {
        let path = format!("target/{target}/ci/goodfetch-{product}");
        workspace.write(&path, contents);
        write!(
            manifest,
            "\n[[file]]\nproduct = \"{product}\"\nkind = \"executable\"\ntarget = \"{target}\"\n\
             name = \"goodfetch-{product}\"\npath = \"{path}\"\n",
        )
        .unwrap();
    }
    workspace.write("dist/files.toml", manifest.as_bytes());
}

fn static_pie(workspace: &Workspace) -> Outcome {
    workspace.xtask(&["static-pie", "--files", "dist/files.toml"])
}

/// The finding of `kind` on each Product's executable for `target`.
fn on_both_products(kind: &str, target: &str) -> Vec<Finding> {
    Vec::from(
        ["collie", "foxhound"]
            .map(|product| finding(kind, &format!("{product}/{target}/goodfetch-{product}"))),
    )
}

#[rstest]
fn static_pie_with_packed_relocations_passes(#[values(MUSL_X64, MUSL_ARM64)] target: &str) {
    let workspace = Workspace::from_fixture("empty");
    write_executables(&workspace, target, &Elf::static_pie().bytes());

    let outcome = static_pie(&workspace);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// What Zig's `cc` links when rustc asks it for a static-PIE: an executable
/// at a fixed base, with no dynamic section.
#[test]
fn executable_linked_at_a_fixed_base_fails() {
    let workspace = Workspace::from_fixture("empty");
    let fixed = Elf {
        kind: ET_EXEC,
        interpreter: false,
        dynamic: None,
    };
    write_executables(&workspace, MUSL_X64, &fixed.bytes());

    let outcome = static_pie(&workspace);

    assert_eq!(outcome.findings(), on_both_products("fixed-base", MUSL_X64));
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::with_an_interpreter(
    Elf { interpreter: true, ..Elf::static_pie() },
    "interpreter",
)]
#[case::needing_a_shared_library(
    Elf { dynamic: Some(vec![(DT_NEEDED, 1), (DT_RELR, 0x2b8)]), ..Elf::static_pie() },
    "shared-library",
)]
#[case::with_relocations_not_packed(
    Elf { dynamic: Some(vec![(DT_FLAGS_1, 0x0800_0001)]), ..Elf::static_pie() },
    "relocations-not-packed",
)]
#[case::with_no_dynamic_section(
    Elf { dynamic: None, ..Elf::static_pie() },
    "relocations-not-packed",
)]
fn position_independent_executable_that_is_no_packed_static_pie_fails(
    #[case] elf: Elf,
    #[case] kind: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    write_executables(&workspace, MUSL_ARM64, &elf.bytes());

    let outcome = static_pie(&workspace);

    assert_eq!(outcome.findings(), on_both_products(kind, MUSL_ARM64));
    assert_eq!(outcome.code, 1);
}

/// Only the static musl build is held to it: the other targets' executables
/// are not read.
#[rstest]
fn executable_of_another_target_is_not_read(
    #[values(
        "x86_64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc"
    )]
    target: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    write_executables(&workspace, target, b"not an ELF image");

    let outcome = static_pie(&workspace);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::no_elf_image(b"not an ELF image".to_vec())]
#[case::cut_short(Elf::static_pie().bytes()[..80].to_vec())]
fn musl_executable_that_cannot_be_read_is_an_error(#[case] contents: Vec<u8>) {
    let workspace = Workspace::from_fixture("empty");
    write_executables(&workspace, MUSL_X64, &contents);

    let outcome = static_pie(&workspace);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
}
