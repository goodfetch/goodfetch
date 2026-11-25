//! The quiet check's readings the trials file records for each timed trial
//! and run under strace: what the machine was when it started, and why it
//! failed the check when it did (#3352).

use serde::{Deserialize, Serialize};

/// The share of [`QUIET_SAMPLE_SECONDS`] each core the run may use must
/// spend idle for the machine to count as quiet: a core another process
/// keeps busy slows the trial on it, whatever the rest of the machine does
/// (#3352). 3 % leaves room for the OS's own housekeeping.
pub(crate) const QUIET_IDLE_SHARE: f64 = 0.97;
/// The seconds over which the quiet check reads how idle the run's cores
/// are (`/proc/stat`): long enough to cover a few of the scheduler's
/// periods, short beside a trial.
pub(crate) const QUIET_SAMPLE_SECONDS: u64 = 3;
/// The CPU pressure (`some avg10` of `/proc/pressure/cpu`, in percent) under
/// which the machine counts as quiet: a share of the last 10 s in which a
/// runnable task waited for a CPU. Under 2 %, nothing queues for the cores.
pub(crate) const QUIET_CPU_PRESSURE: f64 = 2.0;
/// The memory pressure (`full avg10` of `/proc/pressure/memory`, in
/// percent) under which the machine counts as quiet: a share of the last
/// 10 s in which every task stalled on memory, reclaim or swap. Any such
/// stall shows in a trial's time, so the bar is near zero.
pub(crate) const QUIET_MEMORY_PRESSURE: f64 = 0.5;

/// What the quiet check read before a timed trial or a run under strace
/// started: of the run's cores where the OS gives pressure files (Linux
/// with PSI), else the 1-minute load.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QuietReading {
    Cores(Cores),
    Load(f64),
}

/// What the machine was when a trial or a counted run started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Started {
    /// The 1-minute load average, where the OS gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) load: Option<f64>,
    /// What the quiet check read once it passed, or once the wait for it
    /// ended; none when the run did not wait for one (a quicker try, or a
    /// run under Callgrind) or the OS gave nothing to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quiet: Option<QuietReading>,
}

/// The quiet check's readings of the cores the run may use.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Cores {
    /// The share of [`QUIET_SAMPLE_SECONDS`] the busiest of the run's cores
    /// spent idle, from 0 to 1.
    pub(crate) least_idle: f64,
    /// `some avg10` of `/proc/pressure/cpu`, in percent.
    pub(crate) cpu_pressure: f64,
    /// `full avg10` of `/proc/pressure/memory`, in percent.
    pub(crate) memory_pressure: f64,
    /// The processes the run started that still run: what the previous
    /// trial left.
    pub(crate) leftovers: usize,
}

impl QuietReading {
    /// Why the machine is not quiet by this reading, each failed reading
    /// named, or none when it is; `load` is the load it must be under,
    /// read only where the OS gives no pressure files.
    pub(crate) fn failed(&self, load: f64) -> Option<String> {
        let failed: Vec<String> = match *self {
            Self::Load(now) => (now >= load)
                .then(|| format!("1-minute load {now:.2}, at or over {load}"))
                .into_iter()
                .collect(),
            Self::Cores(cores) => [
                (cores.least_idle < QUIET_IDLE_SHARE).then(|| {
                    format!(
                        "a core of the run's was idle {:.1} % of {QUIET_SAMPLE_SECONDS} s, under {:.0} %",
                        cores.least_idle * 100.0,
                        QUIET_IDLE_SHARE * 100.0
                    )
                }),
                (cores.cpu_pressure >= QUIET_CPU_PRESSURE).then(|| {
                    format!(
                        "CPU pressure (some avg10) {:.2} %, at or over {QUIET_CPU_PRESSURE} %",
                        cores.cpu_pressure
                    )
                }),
                (cores.memory_pressure >= QUIET_MEMORY_PRESSURE).then(|| {
                    format!(
                        "memory pressure (full avg10) {:.2} %, at or over {QUIET_MEMORY_PRESSURE} %",
                        cores.memory_pressure
                    )
                }),
                (cores.leftovers > 0).then(|| match cores.leftovers {
                    1 => "1 process of the previous trial still runs".to_owned(),
                    n => format!("{n} processes of the previous trial still run"),
                }),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        (!failed.is_empty()).then(|| failed.join("; "))
    }

    /// The check this reading was of, as the report names it; `load` as in
    /// [`Self::failed`].
    pub(crate) fn check(&self, load: f64) -> String {
        match self {
            Self::Load(_) => {
                format!("a 1-minute load under {load}, the OS giving no pressure files")
            }
            Self::Cores(_) => format!(
                "each of the run's cores idle {:.0} % of {QUIET_SAMPLE_SECONDS} s, CPU pressure under {QUIET_CPU_PRESSURE} %, memory pressure under {QUIET_MEMORY_PRESSURE} % and no process left of the previous trial",
                QUIET_IDLE_SHARE * 100.0
            ),
        }
    }
}
