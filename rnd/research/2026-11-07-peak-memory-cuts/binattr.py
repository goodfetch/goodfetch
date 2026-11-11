"""peak-cuts: attributes an executable's bytes, static and resident, to
sections and to crates.

usage: binattr.py <exe> [<pagemap file from sampler.py>]

Static: every symbol's size (nm -S), by section and crate. Resident: the
pages the pagemap marks present, each page's bytes split over the sections
and symbols it overlaps; bytes no symbol covers count as "(no symbol)".
Crate of a Rust symbol: the first path segment that is not core, alloc or std
(so a generic instantiated for a crate's type counts for that crate); C and
C++ symbols by name prefix.
"""
import bisect
import re
import subprocess
import sys
from collections import defaultdict

exe = sys.argv[1]
pmfile = sys.argv[2] if len(sys.argv) > 2 else None
PAGE = 4096

def run(*a):
    return subprocess.run(a, check=True, capture_output=True, text=True).stdout

# Sections with an address (allocated).
sections = []
for line in run("readelf", "-SW", exe).splitlines():
    m = re.match(r"\s*\[\s*\d+\]\s+(\S+)\s+\S+\s+([0-9a-f]+)\s+([0-9a-f]+)\s+([0-9a-f]+)\s+\S+\s+(\S*)", line)
    if m and int(m[2], 16) and "A" in m[5]:
        sections.append((m[1], int(m[2], 16), int(m[3], 16), int(m[4], 16)))
loads = []
for line in run("readelf", "-lW", exe).splitlines():
    m = re.match(r"\s*LOAD\s+0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+0x[0-9a-f]+\s+0x([0-9a-f]+)", line)
    if m:
        loads.append((int(m[1], 16), int(m[2], 16), int(m[3], 16)))

def off2vaddr(off):
    for o, v, sz in loads:
        if o <= off < o + sz:
            return v + off - o
    return None

C_PREFIX = [
    (r"^(sqlite3|sqlite)", "C: sqlite"),
    (r"^(ZSTD|ZSTDv|HUF_|FSE_|HIST_|ZDICT|COVER|POOL_|ERR_getErrorString)", "C: zstd"),
    (r"^(Brotli|kBrotli|_kBrotli)", "C: brotli"),
    (r"^(_?mi_|_mi_)", "C: mimalloc"),
    (r"^(duckdb|duckdb_)", "C++: duckdb"),
    (r"^(bssl|OPENSSL_|SSL_|EVP_|BN_|EC_|RSA_|X509|ASN1_|CRYPTO_|ERR_|BIO_|PEM_|PKCS|SHA|MD5|AES_|aes_|gcm_|ChaCha|CRYPTO|x25519|ec_|bn_|rsa_|asn1_|ssl_|tls|dtls|HMAC|HKDF|CBB_|CBS_|EVP|DH_|DSA_|ECDSA|ED25519|X25519|sha|md5|chacha|poly1305|boringssl|fiat_|p256|ecp_nistz|bcm_|BCM_|RAND_|NCONF|CONF_|OBJ_|i2d_|d2i_|i2v|v2i|i2s|s2i|i2r|kOpenSSL|v3_|policy|pmbtoken|voprf|HPKE|SPAKE|SLHDSA|MLDSA|MLKEM|KYBER|TRUST_TOKEN|DTLS|EVP_HPKE|ec_GFp|bn_mul|aesni|vpaes|gcm_|ecdsa|hwaes|abi_test)", "C: boringssl"),
    (r"^(icu_|u_|U_|uprv|ucase|ubrk|unorm)", "C: icu"),
    (r"^(re2|RE2)", "C++: re2"),
    (r"^(std::|__cxx|__gnu_cxx|__cxa|operator new|operator delete|typeinfo|vtable for std|_ZNSt|__dynamic_cast)", "C++: libstdc++ (static parts)"),
    (r"^(__rust|rust_|_rust)", "std"),
    (r"^(_init|_fini|_start|__libc|__do_global|frame_dummy|register_tm|deregister_tm|__x86|__cpu|__memcpy|__memset|__memmove|memcpy|memset|memmove|__divti3|__udivti3|__umodti3|__modti3|__mul|__div|__fix|__float|__extend|__trunc|__ashl|__lshr|__ashr|__cmp|__pop|__clz|__ctz|__bswap|__powi|__neg|__sub|__add|__gnu|__TMC|__dso|__GNU|__init|__fini|__bss|__data|_edata|_end|_DYNAMIC|_GLOBAL_OFFSET_TABLE_|__ehdr|__executable|__tls|_IO_|__stack_chk)", "libc/compiler-rt glue"),
]
LOCK = "/Users/jb/Dev/goodfetch/.claude/worktrees/confident-edison-3e91c2/Cargo.lock"
CRATES = {m[1].replace("-", "_") for m in re.finditer(r'^name = "([^"]+)"', open(LOCK).read(), re.M)}
RUST_HASH = re.compile(r"::h[0-9a-f]{16}$")

def crate_of(name):
    is_rust = name.startswith("RUST ")
    base = name[5:] if is_rust else name
    if is_rust:
        for m in re.finditer(r"([A-Za-z_][A-Za-z0-9_]*)::", base):
            if m[1] in CRATES and m[1] not in ("duckdb",):
                return m[1]
            if m[1] == "duckdb":
                return "duckdb (Rust crate)"
        return "std/core/alloc"
    for prefix, c in (("typeinfo", None), ("vtable for ", None), ("non-virtual thunk to ", None),
                      ("guard variable for ", None), ("construction vtable for ", None)):
        if base.startswith(prefix):
            base = base[len(prefix):].lstrip()
            if base.startswith("name for "):
                base = base[len("name for "):]
    if base.startswith(("duckdb", "std::_Function_handler<void (duckdb", "std::_Sp_counted_ptr_inplace<duckdb")) or "duckdb::" in base[:200]:
        return "C++: duckdb"
    if base.startswith(("std::", "__gnu_cxx::", "__cxxabiv1")):
        return "C++: libstdc++ (static parts)"
    for pat, c in C_PREFIX:
        if re.match(pat, base):
            return c
    if "::" in base:
        return "C++ (other)"
    return "(other C symbol)"

syms = []
raw_lines = run("nm", "-S", "--defined-only", exe).splitlines()
dem_lines = run("nm", "-S", "--defined-only", "-C", exe).splitlines()
assert len(raw_lines) == len(dem_lines)
RUST_MANGLED = re.compile(r"^(_R|_ZN.*17h[0-9a-f]{16}E)")
for line, dline in zip(raw_lines, dem_lines):
    p = line.split(" ", 3)
    d = dline.split(" ", 3)
    if len(p) == 4 and p[2] in "tTrRdDbBwWvV":
        a, s = int(p[0], 16), int(p[1], 16)
        if s:
            name = d[3]
            if RUST_MANGLED.match(p[3]):
                name = "RUST " + name
            syms.append((a, s, name))
syms.sort()
starts = [a for a, _, _ in syms]

def section_of(v):
    for n, a, o, sz in sections:
        if a <= v < a + sz:
            return n, a + sz
    return None, None

static_sec = defaultdict(int)
static_crate = defaultdict(int)
for n, a, o, sz in sections:
    static_sec[n] += sz
for a, s, n in syms:
    sec, _ = section_of(a)
    if sec in (".text", ".rodata", ".data.rel.ro", ".data", ".bss", ".tbss", ".tdata"):
        static_crate[(sec, crate_of(n))] += s

def overlap(lo, hi):
    """Bytes of [lo, hi) by crate, and the uncovered rest."""
    out = defaultdict(int)
    i = bisect.bisect_right(starts, lo) - 1
    i = max(i, 0)
    covered = 0
    while i < len(syms) and syms[i][0] < hi:
        a, s, n = syms[i]
        x, y = max(a, lo), min(a + s, hi)
        if y > x:
            out[crate_of(n)] += y - x
            covered += y - x
        i += 1
    out["(no symbol)"] += max(0, (hi - lo) - covered)
    return out

res_sec = defaultdict(int)
res_crate = defaultdict(int)
if pmfile:
    for line in open(pmfile):
        off_hex, perm, bits = line.split()
        base = int(off_hex, 16)
        for i, b in enumerate(bits):
            if b != "1":
                continue
            off = base + i * PAGE
            v = off2vaddr(off)
            if v is None:
                res_sec["(outside LOAD)"] += PAGE
                continue
            lo, hi = v, v + PAGE
            done = 0
            for n, a, o, sz in sections:
                x, y = max(a, lo), min(a + sz, hi)
                if y > x:
                    res_sec[n] += y - x
                    done += y - x
                    if n in (".text", ".rodata", ".data.rel.ro"):
                        for c, k in overlap(x, y).items():
                            res_crate[(n, c)] += k
            if done < PAGE:
                res_sec["(between sections)"] += PAGE - done

MB = 1024 * 1024
print(f"# {exe}")
print("## sections: static MiB, resident MiB")
for n, a, o, sz in sorted(sections, key=lambda s: -s[3]):
    if sz >= 64 * 1024 or res_sec.get(n, 0) >= 64 * 1024:
        print(f"{n:24s} {sz/MB:8.2f} {res_sec.get(n,0)/MB:8.2f}")
for k in ("(between sections)", "(outside LOAD)"):
    if res_sec.get(k):
        print(f"{k:24s} {'':8s} {res_sec[k]/MB:8.2f}")
print(f"{'TOTAL resident':24s} {'':8s} {sum(res_sec.values())/MB:8.2f}")
for sec in (".text", ".rodata", ".data.rel.ro"):
    keys = {c for (s, c) in list(static_crate) + list(res_crate) if s == sec}
    rows = sorted(keys, key=lambda c: -(res_crate.get((sec, c), 0) or static_crate.get((sec, c), 0) / 1000))
    print(f"## {sec} by crate: static MiB, resident MiB")
    for c in rows[:45]:
        st, rs = static_crate.get((sec, c), 0), res_crate.get((sec, c), 0)
        if st >= 128 * 1024 or rs >= 64 * 1024:
            print(f"{c:40s} {st/MB:8.2f} {rs/MB:8.2f}")
