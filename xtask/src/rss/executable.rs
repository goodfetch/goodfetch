//! The executable's own resident pages (#2114): how much of a peak is the
//! measured binary's file-backed pages, its code and read-only data the OS
//! reads from the file, rather than what it allocated. A static musl build
//! (ADR 0024) maps no shared library, so these are all the file pages it
//! has. On Linux, a thread of its own reads the child's `/proc/<pid>/smaps`
//! every [`INTERVAL`] while it runs and keeps the highest sum, over the
//! mappings of the child's own executable, of each one's `Rss` less its
//! `Anonymous` (its copied-on-write pages, which the process wrote and so
//! counts as allocated). Elsewhere nothing is sampled.
#![cfg_attr(
    target_os = "linux",
    expect(
        clippy::disallowed_methods,
        reason = "xtask is synchronous: the sampler reads /proc with blocking calls on a thread of its own, which stops when the measured child is waited for (STYLE.md)"
    )
)]

#[cfg(any(target_os = "linux", test))]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::sync::mpsc;
#[cfg(target_os = "linux")]
use std::thread::JoinHandle;
#[cfg(target_os = "linux")]
use std::time::Duration;

/// How often the sampler reads the child's mappings.
#[cfg(target_os = "linux")]
const INTERVAL: Duration = Duration::from_millis(20);

#[cfg(any(target_os = "linux", test))]
/// The file-backed resident bytes of `executable`'s own mappings in
/// `smaps`, the text of a `/proc/<pid>/smaps`: each mapping's `Rss` less
/// its `Anonymous`, summed over the mappings whose path is `executable`.
pub(crate) fn file_backed_bytes(smaps: &str, executable: &Path) -> u64 {
    let mut total_kib = 0;
    let mut ours = false;
    let mut resident = 0;
    let mut anonymous = 0;
    let mut close = |ours: bool, resident: u64, anonymous: u64| {
        if ours {
            total_kib += resident.saturating_sub(anonymous);
        }
    };
    for line in smaps.lines() {
        if let Some(path) = mapping_path(line) {
            close(ours, resident, anonymous);
            ours = Path::new(path) == executable;
            (resident, anonymous) = (0, 0);
        } else if let Some(kib) = field(line, "Rss:") {
            resident = kib;
        } else if let Some(kib) = field(line, "Anonymous:") {
            anonymous = kib;
        }
    }
    close(ours, resident, anonymous);
    total_kib * 1024
}

#[cfg(any(target_os = "linux", test))]
/// A mapping's header line, `<start>-<end> <perms> <offset> <dev> <inode>
/// [<path>]`, gives its path, empty for an anonymous mapping; any other
/// line gives none.
fn mapping_path(line: &str) -> Option<&str> {
    let mut fields = line.splitn(6, ' ');
    let range = fields.next()?;
    let (start, end) = range.split_once('-')?;
    if !start
        .bytes()
        .chain(end.bytes())
        .all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    // Permissions, offset, device and inode, then the padded path.
    for _ in 0..4 {
        fields.next()?;
    }
    Some(fields.next().map_or("", str::trim))
}

#[cfg(any(target_os = "linux", test))]
/// A `<name> <n> kB` line's figure in KiB.
fn field(line: &str, name: &str) -> Option<u64> {
    line.strip_prefix(name)?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse()
        .ok()
}

/// A running sampler of one child's executable pages.
pub(crate) struct Sampler {
    #[cfg(target_os = "linux")]
    stop: mpsc::Sender<()>,
    #[cfg(target_os = "linux")]
    thread: JoinHandle<Option<u64>>,
}

impl Sampler {
    /// Starts sampling the process `pid`, a child that already started its
    /// executable.
    #[cfg(target_os = "linux")]
    pub(crate) fn start(pid: u32) -> Self {
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let executable = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
            let mut highest = None;
            loop {
                if let Ok(smaps) = std::fs::read_to_string(format!("/proc/{pid}/smaps")) {
                    let bytes = file_backed_bytes(&smaps, &executable);
                    highest = Some(highest.map_or(bytes, |highest: u64| highest.max(bytes)));
                }
                match stopped.recv_timeout(INTERVAL) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            highest
        });
        Self { stop, thread }
    }

    /// Starts nothing: this OS has no `/proc` to sample.
    #[cfg(not(target_os = "linux"))]
    pub(crate) fn start(_pid: u32) -> Self {
        Self {}
    }

    /// Stops sampling and returns the highest sample, none when no sample
    /// was read or the OS has none.
    #[cfg(target_os = "linux")]
    pub(crate) fn finish(self) -> Option<u64> {
        // A sampler that already returned has dropped its receiver.
        let _ = self.stop.send(());
        self.thread.join().ok().flatten()
    }

    /// Stops sampling and returns the highest sample, none when no sample
    /// was read or the OS has none.
    #[cfg(not(target_os = "linux"))]
    #[expect(
        clippy::unused_self,
        reason = "the sampler's interface is the same on every OS"
    )]
    pub(crate) fn finish(self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests;
