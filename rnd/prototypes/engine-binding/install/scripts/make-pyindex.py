"""Builds PEP 503 "simple" indexes under out/pyindex/<variant>/simple/ from the built wheels.

Variants (what a release would publish):
  full       native: abi3 + cp313 + cp314t wheels + sdist;  rpc: per-libc py3-none wheels + sdist
  abi3sdist  native: abi3 wheels + sdist only;              rpc: ONE static-musl wheel per Linux arch,
                                                                 dual-tagged manylinux+musllinux (uv style)
  abi3       native: abi3 wheels only (no sdist);           rpc: same dual-tagged wheels, no sdist
"""
import hashlib
import html
import json
import shutil
import subprocess
import sys
from pathlib import Path

OUT = Path(sys.argv[1]).resolve()
IDX = OUT / "pyindex"
FILES = IDX / "files"
if IDX.exists():
    shutil.rmtree(IDX)
FILES.mkdir(parents=True)

native = sorted((OUT / "wheels/native").glob("*.whl"))
rpc = sorted((OUT / "wheels/rpc").glob("*.whl"))
sdists = {p.name.split("-")[0]: p for p in (OUT / "sdist").glob("*.tar.gz")}

# uv-style: retag the static musl engined wheel so one file serves glibc and musl.
dual_dir = OUT / "wheels/rpc-dual"
if dual_dir.exists():
    shutil.rmtree(dual_dir)
dual_dir.mkdir()
for w in rpc:
    if "musllinux_1_2_" not in w.name:
        continue
    arch = w.name.rsplit("musllinux_1_2_", 1)[1].removesuffix(".whl")
    tag = f"manylinux_2_17_{arch}.manylinux2014_{arch}.musllinux_1_2_{arch}"
    tmp = dual_dir / w.name
    shutil.copy(w, tmp)
    subprocess.run(["uvx", "wheel", "tags", "--remove", "--platform-tag", tag, str(tmp)], check=True,
                   capture_output=True)
dual = sorted(dual_dir.glob("*.whl"))
mac_rpc = [w for w in rpc if "macosx" in w.name]
abi3 = [w for w in native if "-abi3-" in w.name]

variants = {
    "full": {"automan-native": native + [sdists["automan_native"]], "automan-rpc": rpc + [sdists["automan_rpc"]]},
    "abi3sdist": {"automan-native": abi3 + [sdists["automan_native"]], "automan-rpc": dual + mac_rpc + [sdists["automan_rpc"]]},
    "abi3": {"automan-native": abi3, "automan-rpc": dual + mac_rpc},
}

sizes = {}
for variant, projects in variants.items():
    root = IDX / variant / "simple"
    root.mkdir(parents=True)
    (root / "index.html").write_text("".join(f'<a href="{p}/">{p}</a><br>\n' for p in projects))
    for project, files in projects.items():
        (root / project).mkdir()
        links = []
        for f in files:
            dst = FILES / f.name
            if not dst.exists():
                shutil.copy(f, dst)
            digest = hashlib.sha256(dst.read_bytes()).hexdigest()
            sizes[f.name] = dst.stat().st_size
            links.append(f'<a href="../../../files/{html.escape(f.name)}#sha256={digest}">{html.escape(f.name)}</a><br>')
        (root / project / "index.html").write_text("<html><body>\n" + "\n".join(links) + "\n</body></html>\n")
    print(variant, {k: len(v) for k, v in projects.items()})
(IDX / "sizes.json").write_text(json.dumps(sizes, indent=1))
