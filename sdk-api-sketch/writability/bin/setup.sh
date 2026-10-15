#!/bin/bash
# Builds the work folder the writability runs use: a stub-only copy of the sketch at eb1e35b, and
# the checkers' environments with the versions check.sh pins. Usage: setup.sh [WORK]
# WORK defaults to $WRITABILITY_WORK. Nothing here is committed; the runs are, from bin/sync.sh.
set -u
WORK=${1:-$WRITABILITY_WORK}
REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
SKETCH=$REPO/sdk-api-sketch
mkdir -p "$WORK"/{sketch,gf,ld,ld3,env/py,env/rust/examples,env/rust/src}

# The stubs and their doc comments only: no scenarios, snippets, README or leaders' programs.
git -C "$REPO" archive eb1e35b \
  sdk-api-sketch/typescript/{collie-client,collie,foxhound} \
  sdk-api-sketch/python/goodfetch_{collie_client,collie,foxhound} \
  sdk-api-sketch/rust/{collie-client,collie,foxhound} | tar -x -C "$WORK/sketch" --strip-components=1
chmod -R a-w "$WORK/sketch"

cp "$SKETCH"/typescript/package{,-lock}.json "$WORK/gf/"
cp "$SKETCH"/scenarios/leaders/package{,-lock}.json "$WORK/ld/"
cp "$SKETCH"/scenarios/leaders/stagehand3/package{,-lock}.json "$WORK/ld3/"
for d in gf ld ld3; do
  (cd "$WORK/$d" && PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --no-audit --no-fund --silent \
    && npm install --no-save --no-audit --no-fund --silent typescript@7.0.2 && echo "npm $d ok")
done

# The Rust workspace of the sketch, with its examples left out: each run's program is added as one.
cp "$SKETCH/rust/Cargo.toml" "$WORK/env/rust/"
cp "$SKETCH/rust/src/lib.rs" "$WORK/env/rust/src/"
cp -R "$SKETCH"/rust/{collie-client,collie,foxhound} "$WORK/env/rust/"
python3 - "$WORK/env/rust/Cargo.toml" <<'EOF'
import re, sys
p = sys.argv[1]
s = open(p).read()
s = re.sub(r"\n# The testing kits.*?\ntest = true\n", "\n", s, flags=re.S)
open(p, "w").write(s)
EOF
(cd "$WORK/env/rust" && cargo check -q --workspace --all-features && echo "rust ok")

py() {
  local name=$1
  shift
  (cd "$WORK/env/py" && uv venv -q --python 3.11 "$name" && VIRTUAL_ENV=$WORK/env/py/$name uv pip install -q "$@" pyright==1.1.414 \
    && echo "py $name ok")
}
py gf pydantic==2.14.1 fastapi==0.143.1 pytest==9.1.1 playwright==1.64.0 mypy==2.4.0
py pw playwright==1.64.0
py scrapy scrapy==2.19.0 pytest==9.1.1 fastapi==0.143.1 uvicorn==0.54.0
py scrapypw scrapy==2.19.0 scrapy-playwright==0.0.48
py crawlee "crawlee[pydantic-ai,playwright]==1.11.0"
py bu browser-use==0.13.11
echo "setup done"
