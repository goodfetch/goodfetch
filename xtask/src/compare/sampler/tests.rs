use super::{Sample, Sampler, figures};

fn sample(at: f64, resident_bytes: u64, cpu_seconds: f64) -> Sample {
    Sample {
        at,
        resident_bytes,
        private_bytes: resident_bytes / 4,
        executable_bytes: None,
        pss_bytes: Some(resident_bytes / 2),
        cpu_seconds,
        threads: 3,
        open_files: 9,
    }
}

/// Samples whose tree's PSS is each of `pss`, in turn.
fn samples_of_pss(pss: &[Option<u64>]) -> Vec<Sample> {
    pss.iter()
        .zip(0_u32..)
        .map(|(&pss_bytes, index)| Sample {
            pss_bytes,
            ..sample(f64::from(index) * 0.1, 500, 0.0)
        })
        .collect()
}

/// The tree's peak PSS is its highest sample's, as its peak RSS is.
#[test]
fn the_peak_pss_is_the_highest_samples() {
    let sampled = figures(&samples_of_pss(&[Some(70), Some(90), Some(80)]), &[]);

    assert_eq!(sampled.peak_pss_bytes, Some(90));
}

/// A sample that could not read a live process's PSS summed less than the
/// tree's: the trial has no peak PSS rather than one that left it out.
#[test]
fn a_sample_without_a_process_pss_leaves_the_trial_without_a_peak_pss() {
    let sampled = figures(&samples_of_pss(&[Some(70), None, Some(80)]), &[]);

    assert_eq!(sampled.peak_pss_bytes, None);
}

/// A trial that ended before its first sample read no PSS: none, not a
/// tree of no size.
#[test]
fn a_trial_without_a_sample_has_no_peak_pss() {
    assert_eq!(figures(&[], &[]).peak_pss_bytes, None);
}

/// The fixture: a shell whose child, `tail`, holds the last `HELD_BYTES` of
/// its input in memory until the input ends. The shell itself is small, so
/// the tree's PSS is the child's known size and little more, which a
/// reader of the shell's process alone would miss.
#[cfg(target_os = "linux")]
const HELD_BYTES: u64 = 64 * 1024 * 1024;

/// A shell whose child, `tail`, holds [`HELD_BYTES`] it read: the shell,
/// its input still open, and the sampler's last sample of the tree.
#[cfg(target_os = "linux")]
fn held_tree(executable_pages: bool) -> Sample {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    // `; :` keeps the shell as `tail`'s parent rather than replaced by it.
    let mut shell = Command::new("sh")
        .args(["-c", &format!("tail -c {HELD_BYTES}; :")])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = shell.stdin.take().unwrap();
    // Twice what it holds, so every byte it holds was read before the last
    // write returns, whatever the pipe still buffers.
    let block = vec![b'x'; 1024 * 1024];
    for _ in 0..2 * HELD_BYTES / block.len() as u64 {
        input.write_all(&block).unwrap();
    }
    let sampler = Sampler::start(std::time::Instant::now(), executable_pages);

    sampler.follow(shell.id());
    std::thread::sleep(std::time::Duration::from_millis(
        4 * super::INTERVAL_MILLISECONDS,
    ));
    let taken = sampler.finish().unwrap();
    drop(input);
    shell.wait().unwrap();
    *taken.last().unwrap()
}

/// The sampler sums the PSS of the followed process and of every process
/// under it: a child of a known size counts whole.
#[cfg(target_os = "linux")]
#[test]
fn a_child_of_known_size_counts_in_the_trees_pss() {
    let last = held_tree(false);

    let pss = last.pss_bytes.unwrap();
    assert!(
        (HELD_BYTES..HELD_BYTES + HELD_BYTES / 4).contains(&pss) && pss <= last.resident_bytes,
        "{last:?}"
    );
}

/// What the child allocated to hold its input is private memory, and the
/// executables' pages the tree maps from their files are not.
#[cfg(target_os = "linux")]
#[test]
fn a_child_allocation_counts_in_the_tree_private_memory() {
    let last = held_tree(false);

    assert!(
        (HELD_BYTES..HELD_BYTES + HELD_BYTES / 4).contains(&last.private_bytes)
            && last.private_bytes < last.resident_bytes,
        "{last:?}"
    );
    assert_eq!(last.executable_bytes, None, "not asked for: {last:?}");
}

/// Asked for, the followed process's executable's own pages are read: the
/// shell's code it runs is resident and backed by its file.
#[cfg(target_os = "linux")]
#[test]
fn the_followed_process_executable_pages_are_read_when_asked_for() {
    let last = held_tree(true);

    let executable = last.executable_bytes.unwrap();
    assert!(
        executable > 0 && executable < last.resident_bytes,
        "{last:?}"
    );
}

/// The peaks of private memory and of the executable's pages are their
/// highest samples'; a trial whose samples read no executable pages has
/// none.
#[test]
fn the_private_and_executable_peaks_are_the_highest_samples() {
    let mut taken = samples();
    taken[2].private_bytes = 900;
    taken[1].executable_bytes = Some(40);
    taken[3].executable_bytes = Some(70);

    let sampled = figures(&taken, &[]);

    assert_eq!(
        (sampled.peak_private_bytes, sampled.peak_executable_bytes),
        (900, Some(70))
    );
    assert_eq!(figures(&samples(), &[]).peak_executable_bytes, None);
}

/// Six samples 0.1 s apart: 100 bytes resident for the first half second,
/// 300 at the last, and 0.4 s of CPU in the half second from the first to
/// the fifth sample.
fn samples() -> Vec<Sample> {
    vec![
        sample(0.0, 100, 0.0),
        sample(0.1, 100, 0.1),
        sample(0.2, 100, 0.2),
        sample(0.3, 100, 0.3),
        sample(0.4, 100, 0.4),
        sample(0.5, 300, 0.45),
    ]
}

#[test]
fn the_peak_and_time_weighted_mean_come_from_the_samples() {
    let sampled = figures(&samples(), &[]);

    // 0.4 s at 100 bytes, then 0.1 s rising to 300: (40 + 20) / 0.5.
    assert_eq!(
        (sampled.samples, sampled.peak_bytes, sampled.average_bytes),
        (6, 300, 120)
    );
    assert_eq!((sampled.peak_threads, sampled.peak_open_files), (3, 9));
}

/// The busiest window of five samples used 0.4 s of CPU in 0.4 s: one
/// core.
#[test]
fn the_peak_cores_are_the_busiest_windows_cpu_over_its_time() {
    let sampled = figures(&samples(), &[]);

    assert!(
        sampled
            .peak_cores
            .is_some_and(|cores| (cores - 1.0).abs() < 1e-9),
        "{:?}",
        sampled.peak_cores
    );
}

/// A milestone reads the first sample at or after its time; one past the
/// last sample reads the last.
#[test]
fn a_milestone_reads_the_tree_when_that_many_pages_were_answered() {
    let sampled = figures(&samples(), &[(1, 0.05), (2, 0.45), (3, 9.0)]);

    assert_eq!(sampled.at_pages, vec![(1, 100), (2, 300), (3, 300)]);
}

/// A tree whose process is gone, a tool that exited or one killed before
/// its resume starts, gives no sample: none reads as nothing resident.
#[cfg(target_os = "linux")]
#[test]
fn a_tree_with_no_live_process_gives_no_sample() {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let sampler = Sampler::start(std::time::Instant::now(), false);

    sampler.follow(pid);
    std::thread::sleep(std::time::Duration::from_millis(
        4 * super::INTERVAL_MILLISECONDS,
    ));

    assert_eq!(sampler.finish().map(|samples| samples.len()), Some(0));
}

/// Fewer samples than one window: the crawl ended before a peak of cores
/// could be read.
#[test]
fn a_crawl_shorter_than_one_window_has_no_peak_cores() {
    let sampled = figures(&samples()[..3], &[]);

    assert_eq!(sampled.peak_cores, None);
}

/// A tool that exited but is not yet waited for is a zombie: `/proc` still
/// lists it with nothing resident, and it gives no sample either.
#[cfg(target_os = "linux")]
#[test]
fn an_exited_process_not_yet_waited_for_gives_no_sample() {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    let gone = std::path::PathBuf::from(format!("/proc/{pid}/stat"));
    while !std::fs::read_to_string(&gone).is_ok_and(|stat| stat.contains(") Z ")) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let sampler = Sampler::start(std::time::Instant::now(), false);

    sampler.follow(pid);
    std::thread::sleep(std::time::Duration::from_millis(
        4 * super::INTERVAL_MILLISECONDS,
    ));
    let taken = sampler.finish().map(|samples| samples.len());
    child.wait().unwrap();

    assert_eq!(taken, Some(0));
}
