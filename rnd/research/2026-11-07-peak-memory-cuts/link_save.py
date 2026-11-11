"""peak-cuts: copies the inputs of the ld.lld command Zig printed into
<save>/in and writes <save>/args (one argument per line, inputs rewritten,
the output path replaced by @OUT@).

usage: link_save.py <save>
"""
import os
import pathlib
import shutil
import sys

save = pathlib.Path(sys.argv[1])
cwd = (save / "cwd").read_text().strip()
lines = [l for l in (save / "zig-link.stderr").read_text().splitlines() if l.startswith("ld.lld ")]
assert lines, "Zig printed no ld.lld command"
args = lines[-1].split(" ")[1:]
inp = save / "in"
inp.mkdir(exist_ok=True)


def expand(a):
    # A response file: inline its arguments (one per line, as rustc writes them).
    if a.startswith("@") and os.path.isfile(os.path.join(cwd, a[1:])):
        return [x for x in open(os.path.join(cwd, a[1:])).read().split("\n") if x]
    return [a]


flat = [x for a in args for x in expand(a)]
out, n = [], 0
skip_next = False
for i, a in enumerate(flat):
    if skip_next:
        skip_next = False
        out.append("@OUT@")
        continue
    if a == "-o":
        out.append(a)
        skip_next = True
        continue
    p = a if os.path.isabs(a) else os.path.join(cwd, a)
    if not a.startswith("-") and os.path.isfile(p):
        n += 1
        dst = inp / f"{n:04d}-{os.path.basename(a)}"
        shutil.copy2(p, dst)
        out.append(str(dst))
    else:
        out.append(a)
(save / "args").write_text("\n".join(out) + "\n")
print(f"link_save: {n} inputs saved, {len(out)} arguments", file=sys.stderr)
