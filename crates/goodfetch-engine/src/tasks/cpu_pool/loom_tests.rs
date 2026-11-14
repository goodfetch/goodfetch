//! The CPU pool under loom (ADR 0060): every job queued runs, whatever the
//! interleaving of a submitter with a thread that ends or waits. A job left
//! behind shows as loom's deadlock: the model's thread waits for an answer
//! no thread of the pool will send.

use std::time::Duration;

use loom::sync::mpsc;

use super::{CpuPool, Job};

/// A job that answers `number` on `answers`.
fn answering(answers: &mpsc::Sender<u8>, number: u8) -> Job {
    let answers = answers.clone();
    Box::new(move || {
        answers
            .send(number)
            .expect("the model's thread waits for every answer")
    })
}

/// Takes `count` answers, in any order.
fn answered(answers: &mpsc::Receiver<u8>, count: usize) -> Vec<u8> {
    let mut numbers: Vec<u8> = (0..count)
        .map(|_| answers.recv().expect("a job answers"))
        .collect();
    numbers.sort_unstable();
    numbers
}

#[test]
fn a_job_queued_as_the_last_thread_ends_still_runs() {
    loom::model(|| {
        // No keep-alive: the pool's one thread ends as soon as it finds the
        // queue empty, racing the second submission.
        let pool = CpuPool::new("loom", 1, Duration::ZERO);
        let (answers, answer) = mpsc::channel();
        pool.run(answering(&answers, 1))
            .expect("loom gives every thread");
        pool.run(answering(&answers, 2))
            .expect("loom gives every thread");
        assert_eq!(answered(&answer, 2), [1, 2]);
    });
}

#[test]
fn an_idle_thread_takes_the_next_job_and_ends_with_the_pool() {
    loom::model(|| {
        // loom never times a wait out: the thread stays idle until a job
        // or the pool's end wakes it.
        let pool = CpuPool::new("loom", 1, Duration::from_secs(1));
        let (answers, answer) = mpsc::channel();
        pool.run(answering(&answers, 1))
            .expect("loom gives every thread");
        assert_eq!(answered(&answer, 1), [1]);
        pool.run(answering(&answers, 2))
            .expect("loom gives every thread");
        assert_eq!(answered(&answer, 1), [2]);
        drop(pool);
    });
}

#[test]
fn two_submitters_on_one_thread_both_get_their_job_run() {
    loom::model(|| {
        let pool = loom::sync::Arc::new(CpuPool::new("loom", 1, Duration::ZERO));
        let (answers, answer) = mpsc::channel();
        let other = {
            let pool = loom::sync::Arc::clone(&pool);
            let job = answering(&answers, 2);
            loom::thread::spawn(move || pool.run(job).expect("loom gives every thread"))
        };
        pool.run(answering(&answers, 1))
            .expect("loom gives every thread");
        assert_eq!(answered(&answer, 2), [1, 2]);
        other.join().expect("the submitter does not panic");
    });
}
