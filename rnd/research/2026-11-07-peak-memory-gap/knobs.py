"""peak-gap (measurement only, reverted after the build): Tokio runtime knobs
read from the environment in the executable's runtime builder, absent = today's
defaults: PEAKGAP_MAX_BLOCKING (max_blocking_threads), PEAKGAP_KEEP_ALIVE_MS
(thread_keep_alive), PEAKGAP_STACK (thread_stack_size, bytes).

usage: knobs.py <worktree>
"""
import sys

p = sys.argv[1] + "/crates/goodfetch-collie/src/executable.rs"
s = open(p).read()
old = """tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()"""
new = """peakgap_knobs(tokio::runtime::Builder::new_multi_thread()
        .enable_all())
        .build()"""
assert s.count(old) == 1, "runtime builder not found"
s = s.replace(old, new) + '''
/// peak-gap (measurement only): Tokio knobs from the environment.
fn peakgap_knobs(b: &mut tokio::runtime::Builder) -> &mut tokio::runtime::Builder {
    let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<usize>().ok());
    if let Some(n) = get("PEAKGAP_MAX_BLOCKING") {
        b.max_blocking_threads(n);
    }
    if let Some(ms) = get("PEAKGAP_KEEP_ALIVE_MS") {
        b.thread_keep_alive(std::time::Duration::from_millis(ms as u64));
    }
    if let Some(n) = get("PEAKGAP_STACK") {
        b.thread_stack_size(n);
    }
    b
}
'''
open(p, "w").write(s)
print("patched", p)
