use super::{Sample, Sampler, figures};

fn sample(at: f64, resident_bytes: u64, cpu_seconds: f64) -> Sample {
    Sample {
        at,
        resident_bytes,
        cpu_seconds,
        threads: 3,
        open_files: 9,
    }
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
    let sampler = Sampler::start(std::time::Instant::now());

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
