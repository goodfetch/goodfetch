use std::cell::RefCell;
use std::sync::mpsc;
use std::time::Duration;

use super::{CpuPool, Job};

/// Long enough for a thread of the pool to start and answer on a loaded
/// machine; a test that passes never waits it out.
const ANSWERED: Duration = Duration::from_secs(10);

/// A job that sends the name of the thread it runs on.
fn naming(names: &mpsc::Sender<Option<String>>) -> Job {
    let names = names.clone();
    Box::new(move || {
        let _ = names.send(std::thread::current().name().map(str::to_owned));
    })
}

#[test]
fn a_job_runs_on_a_thread_named_for_the_pool() {
    let pool = CpuPool::new("gf-test-pool", 2, Duration::from_secs(1));
    let (tell, told) = mpsc::channel();

    pool.run(naming(&tell)).unwrap();

    assert_eq!(
        told.recv_timeout(ANSWERED),
        Ok(Some("gf-test-pool".to_owned()))
    );
}

/// Four jobs that each wait for their release, on a pool of at most two
/// threads: two run, two queue, and no third thread starts.
#[test]
fn no_more_threads_than_the_most_run_at_once() {
    let pool = CpuPool::new("gf-test-pool", 2, Duration::from_secs(1));
    let (started, starts) = mpsc::channel();
    let mut releases = Vec::new();
    for job in 0..4 {
        let (release, go_on) = mpsc::channel::<()>();
        releases.push(release);
        let started = started.clone();
        pool.run(Box::new(move || {
            let _ = started.send(job);
            let _ = go_on.recv();
        }))
        .unwrap();
    }
    let first_two = [
        starts.recv_timeout(ANSWERED).unwrap(),
        starts.recv_timeout(ANSWERED).unwrap(),
    ];

    let (threads, queued) = {
        let state = pool.shared.lock();
        (state.threads, state.queue.len())
    };
    drop(releases);
    let mut all = first_two.to_vec();
    all.extend([
        starts.recv_timeout(ANSWERED).unwrap(),
        starts.recv_timeout(ANSWERED).unwrap(),
    ]);
    all.sort_unstable();

    assert_eq!((threads, queued, all), (2, 2, vec![0, 1, 2, 3]));
}

thread_local! {
    /// Told when the thread it belongs to ends.
    static ON_EXIT: RefCell<Option<Ends>> = const { RefCell::new(None) };
}

/// Sends once its thread ends, as the thread's locals drop.
struct Ends(mpsc::Sender<()>);

impl Drop for Ends {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// A thread that ran its job and finds nothing more to run within its
/// keep-alive ends, and the next job starts a thread again.
#[test]
fn a_thread_ends_once_it_waited_its_keep_alive_and_another_starts_on_the_next_job() {
    let pool = CpuPool::new("gf-test-pool", 1, Duration::from_millis(20));
    let (ended, ends) = mpsc::channel();
    pool.run(Box::new(move || {
        ON_EXIT.with(|on_exit| *on_exit.borrow_mut() = Some(Ends(ended)));
    }))
    .unwrap();

    let first_ended = ends.recv_timeout(ANSWERED);
    let (tell, told) = mpsc::channel();
    pool.run(naming(&tell)).unwrap();

    assert_eq!(
        (first_ended, told.recv_timeout(ANSWERED)),
        (Ok(()), Ok(Some("gf-test-pool".to_owned())))
    );
}
