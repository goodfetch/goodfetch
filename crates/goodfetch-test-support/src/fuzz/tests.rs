#![expect(
    clippy::disallowed_methods,
    reason = "these tests build their target folders outside any async worker"
)]

use std::path::{Path, PathBuf};

use rstest::rstest;

use super::{FuzzReplayError, Replayed, replay_fuzz_target};

/// The fixture target: a one-frame length-prefixed decoder's seeds and the
/// crasher that found its bug.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fuzz/fixtures/length_prefix")
}

/// The frame's payload: the bytes after the length byte, as many as it
/// says. The bug: it trusts the length byte.
fn decode_trusting_the_length(data: &[u8]) -> Option<&[u8]> {
    let (&length, rest) = data.split_first()?;
    Some(&rest[..usize::from(length)])
}

/// The same decoder with the bug fixed: a length past the end is no frame.
fn decode(data: &[u8]) -> Option<&[u8]> {
    let (&length, rest) = data.split_first()?;
    rest.get(..usize::from(length))
}

fn buggy_body(data: &[u8]) {
    let _ = decode_trusting_the_length(data);
}

fn fixed_body(data: &[u8]) {
    let _ = decode(data);
}

#[test]
fn a_committed_crasher_fails_the_replay_naming_its_file() {
    let target = fixture();

    let replayed = replay_fuzz_target(&target, buggy_body);

    assert_eq!(
        replayed,
        Err(FuzzReplayError::Crashed {
            files: vec![target.join("crashers/crash-length past the end é")],
        })
    );
}

#[test]
fn a_fixed_bug_replays_every_seed_and_crasher() {
    let target = fixture();

    let replayed = replay_fuzz_target(&target, fixed_body);

    assert_eq!(
        replayed,
        Ok(Replayed {
            files: vec![
                target.join("seeds/empty frame"),
                target.join("seeds/three bytes é"),
                target.join("crashers/crash-length past the end é"),
            ],
        })
    );
}

/// How a test's own target folder holds a folder.
#[derive(Clone, Copy)]
enum Folder {
    Missing,
    Empty,
    OneFile,
}

/// A target folder in `root` with its seeds and crashers folders as given;
/// a folder's one file is a valid frame.
fn target_in(root: &Path, seeds: Folder, crashers: Folder) -> PathBuf {
    let target = root.join("target é");
    for (name, folder) in [("seeds", seeds), ("crashers", crashers)] {
        let path = target.join(name);
        match folder {
            Folder::Missing => {}
            Folder::Empty => std::fs::create_dir_all(&path).unwrap(),
            Folder::OneFile => {
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join("one frame é"), b"\x01a").unwrap();
            }
        }
    }
    target
}

#[rstest]
#[case::missing(Folder::Missing)]
#[case::empty(Folder::Empty)]
fn a_target_without_crashers_replays_its_seeds(#[case] crashers: Folder) {
    let root = tempfile::tempdir().unwrap();
    let target = target_in(root.path(), Folder::OneFile, crashers);

    let replayed = replay_fuzz_target(&target, buggy_body);

    assert_eq!(
        replayed,
        Ok(Replayed {
            files: vec![target.join("seeds/one frame é")],
        })
    );
}

#[rstest]
#[case::missing(Folder::Missing)]
#[case::empty(Folder::Empty)]
fn a_target_without_seeds_fails(#[case] seeds: Folder) {
    let root = tempfile::tempdir().unwrap();
    let target = target_in(root.path(), seeds, Folder::OneFile);

    let replayed = replay_fuzz_target(&target, buggy_body);

    assert_eq!(
        replayed,
        Err(FuzzReplayError::NoSeeds {
            folder: target.join("seeds"),
        })
    );
}
