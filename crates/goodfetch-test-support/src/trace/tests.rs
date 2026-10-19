use std::io;
use std::sync::mpsc;
use std::time::SystemTime;

use super::subscriber_writing_to;

/// Sends each write to the test, which reads them after the event.
#[derive(Clone)]
struct Lines(mpsc::Sender<Vec<u8>>);

impl io::Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let _ = self.0.send(bytes.to_owned());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_trace_carries_the_simulated_time_the_closure_reads() {
    let (sender, receiver) = mpsc::channel();
    let writer = Lines(sender);
    let simulated: jiff::Timestamp = "2026-01-05T09:00:00Z".parse().unwrap();
    let subscriber = subscriber_writing_to(
        move || SystemTime::from(simulated),
        move || writer.clone(),
    );

    tracing::subscriber::with_default(subscriber, || tracing::info!("hello"));

    let written: Vec<u8> = receiver.try_iter().flatten().collect();
    let line = String::from_utf8(written).unwrap();
    assert!(line.starts_with("2026-01-05T09:00:00Z "), "{line}");
}
