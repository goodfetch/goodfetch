"""peak-cuts: measurement-only edit of goodfetch-collie's executable::allocator
(reverted after each build, in no commit). One binary, four ways to turn
mimalloc's transparent huge pages off, picked by PEAKCUT_THP at run time:

  unset  as shipped (only the purge delay is set, in main)
  1      mi_option_set(allow_thp, 0) in main, beside the purge delay
         (the decision of #2151 taken literally)
  2      1, plus prctl(PR_SET_THP_DISABLE) in main
  3      mi_option_set(allow_thp, 0) in an .init_array.00100 constructor,
         before mimalloc's own constructor(101) reads its options

usage: thp-modes.py <worktree>
"""
import pathlib
import sys

path = pathlib.Path(sys.argv[1]) / "crates/goodfetch-collie/src/executable/allocator.rs"
src = path.read_text()
old = """pub(super) fn purge_at_once() {
    // SAFETY: `mi_option_set` takes any option and value (it ignores an
    // option out of range), and no other thread of the process runs yet.
    unsafe { mi_option_set(PURGE_DELAY, 0) }
}"""
new = """pub(super) fn purge_at_once() {
    // SAFETY: `mi_option_set` takes any option and value (it ignores an
    // option out of range), and no other thread of the process runs yet.
    unsafe { mi_option_set(PURGE_DELAY, 0) }
    match peakcut_mode() {
        b'1' => unsafe { mi_option_set(PEAKCUT_ALLOW_THP, 0) },
        b'2' => unsafe {
            mi_option_set(PEAKCUT_ALLOW_THP, 0);
            libc::prctl(libc::PR_SET_THP_DISABLE, 1, 0, 0, 0);
        },
        _ => {}
    }
}

/// peak-cuts (measurement only): `mi_option_allow_thp` in mimalloc v3.
const PEAKCUT_ALLOW_THP: c_int = 43;

fn peakcut_mode() -> u8 {
    // SAFETY: getenv on a NUL-terminated name; read before threads start.
    unsafe {
        let p = libc::getenv(c"PEAKCUT_THP".as_ptr());
        if p.is_null() { 0 } else { *p as u8 }
    }
}

extern "C" fn peakcut_constructor() {
    if peakcut_mode() == b'3' {
        // SAFETY: runs before mimalloc's constructor, single-threaded.
        unsafe { mi_option_set(PEAKCUT_ALLOW_THP, 0) }
    }
}

#[used]
#[unsafe(link_section = ".init_array.00100")]
static PEAKCUT_CONSTRUCTOR: extern "C" fn() = peakcut_constructor;"""
assert src.count(old) == 1, "allocator.rs changed; update thp-modes.py"
path.write_text(src.replace(old, new))
print(f"patched {path}")
