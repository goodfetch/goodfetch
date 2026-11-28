//! A Sink that fails as a destination's link does, around the real SQLite
//! Sink: an answer lost after its write landed, and a kill before or after
//! a write lands, after which the engine is gone and nothing more is
//! written.

use goodfetch_sink::{
    Batch, Failure, FailureKind, Primitives, Probe, RunCopy, SqliteSink, Written,
};

/// When the destination's link fails, counted in writes from 1.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fault {
    /// The write numbered so lands, and its answer is lost.
    AnswerLost(usize),
    /// The engine is killed at the write numbered so, after it landed when
    /// `landed`, before it otherwise.
    Killed { write: usize, landed: bool },
}

/// A Sink behind a link that fails once as `fault` says.
pub(crate) struct Faulty<S = SqliteSink> {
    sink: S,
    fault: Fault,
    writes: usize,
    killed: bool,
}

impl<S> Faulty<S> {
    pub(crate) fn new(sink: S, fault: Fault) -> Self {
        Self {
            sink,
            fault,
            writes: 0,
            killed: false,
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn into_inner(self) -> S {
        self.sink
    }
}

/// What the faulty link answers.
#[derive(Debug)]
pub(crate) enum LinkError<E> {
    Destination(E),
    Lost,
    Killed,
}

impl<S: Primitives> Primitives for Faulty<S> {
    type Error = LinkError<S::Error>;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, Self::Error> {
        if self.killed {
            return Err(LinkError::Killed);
        }
        self.sink.probe(run).map_err(LinkError::Destination)
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error> {
        if self.killed {
            return Err(LinkError::Killed);
        }
        self.writes += 1;
        match self.fault {
            Fault::AnswerLost(write) if write == self.writes => {
                self.sink.write(batch).map_err(LinkError::Destination)?;
                Err(LinkError::Lost)
            }
            Fault::Killed { write, landed } if write == self.writes => {
                if landed {
                    self.sink.write(batch).map_err(LinkError::Destination)?;
                }
                self.killed = true;
                Err(LinkError::Killed)
            }
            _ => self.sink.write(batch).map_err(LinkError::Destination),
        }
    }

    fn finish(&mut self, run: &RunCopy) -> Result<Written, Self::Error> {
        if self.killed {
            return Err(LinkError::Killed);
        }
        self.sink.finish(run).map_err(LinkError::Destination)
    }

    fn flush_expired(&mut self, run: &RunCopy) -> Result<Written, Self::Error> {
        if self.killed {
            return Err(LinkError::Killed);
        }
        self.sink.flush_expired(run).map_err(LinkError::Destination)
    }

    fn next_flush_after(&self) -> Option<std::time::Duration> {
        self.sink.next_flush_after()
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        match error {
            LinkError::Destination(error) => self.sink.map_failure(error),
            LinkError::Lost => Failure::new(FailureKind::Transient, "the answer was lost"),
            LinkError::Killed => Failure::new(FailureKind::Transient, "the engine was killed"),
        }
    }
}
