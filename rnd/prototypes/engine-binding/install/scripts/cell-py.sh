#!/bin/sh
# Runs INSIDE a fresh python container (no Rust toolchain). POSIX sh.
# env: CELL PKG(native|rpc) INST(pip|uv|poetry) VARIANT(full|abi3sdist|abi3) OLDPIP(optional) IDX
# Our packages come from the local index; PyPI is an extra index so an sdist fallback can fetch its build
# backend (maturin) exactly as it would for a real user. Neither project name exists on PyPI.
set -u
R=/results/$CELL; mkdir -p "$R"; LOG=$R/log.txt; : > "$LOG"
PROJ=automan-$PKG
URL="$IDX/$VARIANT/simple/"
PY=$(command -v python3)
IDXHOST=${IDX#http://}; IDXHOST=${IDXHOST%%:*}
mkdir -p /w && cd /w
case "$INST" in
  pip) $PY -m venv /venv >> "$LOG" 2>&1 ;;
  uv) uv venv -q --python "$PY" /venv >> "$LOG" 2>&1 ;;
  poetry) poetry init -q -n --name cell --python ">=3.9" >> "$LOG" 2>&1
          poetry config virtualenvs.in-project true
          poetry source add -q --priority=primary local "$URL" >> "$LOG" 2>&1 ;;
esac
if [ -n "${OLDPIP:-}" ]; then
  echo "\$ pip install pip==$OLDPIP  (setup, not measured)" >> "$LOG"
  /venv/bin/python -m pip install -q "pip==$OLDPIP" >> "$LOG" 2>&1
fi
case "$INST" in
  pip) cmd="/venv/bin/python -m pip install --no-cache-dir --disable-pip-version-check --index-url $URL --extra-index-url https://pypi.org/simple/ --trusted-host $IDXHOST $PROJ" ;;
  uv) cmd="uv pip install --no-cache --python /venv/bin/python --index-url $URL --extra-index-url https://pypi.org/simple/ $PROJ" ;;
  poetry) cmd="poetry add --no-cache $PROJ" ;;
esac

rx() { cat /sys/class/net/eth0/statistics/rx_bytes 2>/dev/null || echo 0; }
up() { cut -d' ' -f1 /proc/uptime; }
echo "\$ $cmd" >> "$LOG"
rx0=$(rx); t0=$(up)
sh -c "$cmd" >> "$LOG" 2>&1; irc=$?
t1=$(up); rx1=$(rx)

VPY=/venv/bin/python; [ "$INST" = poetry ] && VPY=/w/.venv/bin/python
case "$PKG" in
  native) code="import automan_native; print(automan_native.hello())" ;;
  rpc) code="import automan_rpc,os; p=automan_rpc.find_engined(); print('binary', p, 'mode', oct(os.stat(p).st_mode & 0o777)); print(automan_rpc.ping())" ;;
esac
echo "--- first use (venv NOT activated, PATH untouched)" >> "$LOG"
$VPY -c "$code" > "$R/test.txt" 2>&1; trc=$?
cat "$R/test.txt" >> "$LOG"

site=$($VPY -c 'import sysconfig;print(sysconfig.get_path("purelib"))' 2>/dev/null)
tags=$(cat "$site"/automan_*.dist-info/WHEEL 2>/dev/null | sed -n 's/^Tag: //p' | tr '\n' ' ')
disk=$(du -skc "$site"/automan_* "$(dirname "$VPY")/engined" 2>/dev/null | tail -1 | cut -f1)
{
  echo "install_rc=$irc"; echo "test_rc=$trc"
  echo "secs=$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.1f", b-a}')"
  echo "rx_bytes=$((rx1 - rx0))"; echo "disk_kb=$disk"; echo "installed=$tags"
  case "$INST" in
    pip) echo "pm_version=pip $($VPY -m pip --version 2>/dev/null | cut -d' ' -f2)" ;;
    uv) echo "pm_version=$(uv --version)" ;;
    poetry) echo "pm_version=$(poetry --version 2>/dev/null)" ;;
  esac
  echo "runtime=$($PY -c 'import sys,sysconfig;print(sys.version.split()[0]+("t" if sysconfig.get_config_var("Py_GIL_DISABLED") else ""))')"
} > "$R/kv"
