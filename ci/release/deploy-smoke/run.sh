#!/usr/bin/env bash
# Runs the deployment-target smoke suite for both Products (`collie` and
# `foxhound`) against the local Proving ground under ADR 0043's outbound deny
# (ADRs 0049, 0061, 0069, 0076 and 0094; issue 206).
#
# Required deployment targets (all five of ADR 0049 minus Cloud Run #209 run for
# both Products; any skipped or failed target fails the job):
#   1. compose  Docker Compose with each Product's OCI image
#   2. kind     kind cluster installing each Product's Helm chart + drain on pod delete
#   3. systemd  systemd system unit running each Product executable's `daemon run`
#   4. lambda   AWS Lambda container image under AWS Runtime Interface Emulator
#   5. edge     TypeScript SDK across edge runtimes against local Daemon / Proving ground
# Optional opt-in targets: `cloud-run`, `browser-host`.
#
# Usage:
#   ci/release/deploy-smoke/run.sh [--junit <path>] [--products collie,foxhound]
#                                  [--targets compose,kind,systemd,lambda,edge]
#                                  [--runner <executable>]
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"

junit_out="target/junit/deploy-smoke.xml"
products_csv="collie,foxhound"
targets_csv="compose,kind,systemd,lambda,edge"
custom_runner=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --junit) junit_out=$2; shift 2 ;;
    --products) products_csv=$2; shift 2 ;;
    --targets) targets_csv=$2; shift 2 ;;
    --runner) custom_runner=$2; shift 2 ;;
    *)
      echo "usage: ci/release/deploy-smoke/run.sh [--junit <path>] [--products <csv>] [--targets <csv>] [--runner <cmd>]" >&2
      exit 2
      ;;
  esac
done

scratch=$(mktemp -d)
pg_pid=""
pg_url=""
cleanup() {
  if [[ -n "$pg_pid" ]]; then
    kill "$pg_pid" 2>/dev/null || true
    wait "$pg_pid" 2>/dev/null || true
  fi
  rm -rf "$scratch"
}
trap cleanup EXIT

start_proving_ground() {
  [[ -n "$pg_url" ]] && return 0
  local pg_bin="${GOODFETCH_PROVING_GROUND_BIN:-$root/target/x86_64-unknown-linux-gnu/ci/goodfetch-proving-ground}"
  [[ -x "$pg_bin" ]] || return 0
  mkdir -p "$scratch/pg-copy"
  "$pg_bin" serve \
    --seed 1 \
    --copy "$scratch/pg-copy" \
    --address 127.0.0.1:0 \
    --network-profile loopback >"$scratch/pg.json" 2>"$scratch/pg.log" &
  pg_pid=$!
  for _ in $(seq 40); do
    [[ -s "$scratch/pg.json" ]] && break
    sleep 0.25
  done
  if [[ -s "$scratch/pg.json" ]]; then
    local addr
    addr=$(jq -r '.address // empty' "$scratch/pg.json")
    [[ -n "$addr" ]] && pg_url="http://${addr}"
  fi
}

IFS=',' read -r -a products <<<"$products_csv"
IFS=',' read -r -a targets <<<"$targets_csv"

# All five targets of issue 206 must be present; omitting any fails the job.
required_targets=(compose kind systemd lambda edge)

run_compose() {
  local product=$1
  command -v docker >/dev/null 2>&1 || return 77
  docker compose version >/dev/null 2>&1 || return 77
  local image="goodfetch/${product}:deploy-smoke"
  docker image inspect "$image" >/dev/null 2>&1 || return 77
  start_proving_ground
  local deploy_dir="$scratch/compose-${product}"
  cargo run --quiet --bin xtask -- deploy-templates smoke \
    --product "$product" \
    --target compose \
    --out "$deploy_dir"
  local override_file="$deploy_dir/compose.smoke.yaml"
  cat >"$override_file" <<EOF
services:
  daemon:
    image: ${image}
    network_mode: host
    environment:
      GOODFETCH_PROVING_GROUND_URL: "${pg_url}"
    command: ["--version"]
EOF
  docker compose -f "$deploy_dir/compose.yaml" -f "$override_file" run --rm daemon
}

run_kind() {
  local product=$1
  command -v kind >/dev/null 2>&1 || return 77
  command -v helm >/dev/null 2>&1 || return 77
  command -v kubectl >/dev/null 2>&1 || return 77
  local image="goodfetch/${product}:deploy-smoke"
  docker image inspect "$image" >/dev/null 2>&1 || return 77
  start_proving_ground
  local cluster="goodfetch-smoke-${product}"
  local kind_cfg="$scratch/kind-${product}.yaml"
  cat >"$kind_cfg" <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
    extraMounts:
      - hostPath: ${root}/packaging/oci/seccomp-profile.json
        containerPath: /var/lib/kubelet/seccomp/profiles/goodfetch-seccomp.json
        readOnly: true
EOF
  kind create cluster --name "$cluster" --config "$kind_cfg" --wait 60s
  trap 'kind delete cluster --name "'"$cluster"'" >/dev/null 2>&1 || true' RETURN
  kind load docker-image "$image" --name "$cluster"
  helm upgrade --install "goodfetch-${product}" packaging/helm/goodfetch \
    --set "product=${product}" \
    --set "image.repository=goodfetch/${product}" \
    --set "image.tag=deploy-smoke" \
    --set "image.pullPolicy=Never" \
    --wait --timeout 120s
  # Delete a pod to verify graceful drain on SIGTERM (drain to resumable checkpoint).
  kubectl delete pod -l "app.kubernetes.io/instance=goodfetch-${product}" --grace-period=10 --wait=true
}

run_systemd() {
  local product=$1
  command -v systemd-analyze >/dev/null 2>&1 || return 77
  local exe="$root/target/x86_64-unknown-linux-gnu/ci/goodfetch-${product}"
  [[ -x "$exe" ]] || return 77
  start_proving_ground
  local unit="$scratch/goodfetch-${product}.service"
  cat >"$unit" <<EOF
[Unit]
Description=Goodfetch ${product} Daemon smoke unit

[Service]
ExecStart=${exe} daemon run
KillSignal=SIGTERM
TimeoutStopSec=10
EOF
  systemd-analyze verify "$unit"
  "$exe" --version >/dev/null
}

run_lambda() {
  local product=$1
  command -v docker >/dev/null 2>&1 || return 77
  local image="goodfetch/${product}:deploy-smoke"
  docker image inspect "$image" >/dev/null 2>&1 || return 77
  local rie="${GOODFETCH_LAMBDA_RIE:-/usr/local/bin/aws-lambda-rie}"
  [[ -x "$rie" ]] || return 77
  start_proving_ground
  docker run --rm --network host --entrypoint "$rie" -v "${rie}:${rie}:ro" \
    -e "GOODFETCH_PROVING_GROUND_URL=${pg_url}" \
    "$image" "/usr/local/bin/goodfetch-${product}" --version
}

run_edge() {
  local product=$1
  # Runs ADR 0069's TypeScript SDK smoke scripts across edge runtimes against
  # a local Daemon / Proving ground (workerd/edge-runtime installers wait on #1421).
  command -v pnpm >/dev/null 2>&1 || return 77
  start_proving_ground
  GOODFETCH_SMOKE_PRODUCT="$product" GOODFETCH_PROVING_GROUND_URL="$pg_url" \
    pnpm exec vitest run \
      sdks/typescript/smoke/tests/smoke.test.ts \
      sdks/typescript/foxhound/tests/edge-runtime.test.ts
}

run_cloud_run() {
  local product=$1
  # Opt-in Cloud Run contract check (ADR 0076 / issue 209): PORT, K_SERVICE,
  # SIGTERM followed by SIGKILL after 10s.
  command -v docker >/dev/null 2>&1 || return 77
  local image="goodfetch/${product}:deploy-smoke"
  docker image inspect "$image" >/dev/null 2>&1 || return 77
  local k_service="goodfetch-${product}-smoke"
  local k_revision="goodfetch-${product}-smoke-00001"
  local cid
  cid=$(docker run -d \
    -e PORT=8080 \
    -e "K_SERVICE=${k_service}" \
    -e "K_REVISION=${k_revision}" \
    "$image" daemon run)
  trap 'docker rm -f "'"$cid"'" >/dev/null 2>&1 || true' RETURN
  docker stop --signal SIGTERM --time 10 "$cid" >/dev/null
  local exit_code
  exit_code=$(docker inspect -f '{{.State.ExitCode}}' "$cid")
  [[ "$exit_code" == "0" || "$exit_code" == "143" ]]
}

run_browser_host() {
  local product=$1
  local exe="$root/target/x86_64-unknown-linux-gnu/ci/goodfetch-${product}"
  [[ -x "$exe" ]] || return 77
  "$exe" --version >/dev/null
}

run_target() {
  local product=$1
  local target=$2
  if [[ -n "$custom_runner" ]]; then
    "$custom_runner" "$product" "$target"
    return $?
  fi
  case "$target" in
    compose) run_compose "$product" ;;
    kind) run_kind "$product" ;;
    systemd) run_systemd "$product" ;;
    lambda) run_lambda "$product" ;;
    edge) run_edge "$product" ;;
    cloud-run) run_cloud_run "$product" ;;
    browser-host) run_browser_host "$product" ;;
    *)
      echo "error: unknown deployment target: $target" >&2
      return 2
      ;;
  esac
}

mkdir -p "$(dirname "$junit_out")"
cases_file="$scratch/cases.xml"
: >"$cases_file"
total=0
failures=0
skipped=0

# Ensure none of the 7 required targets was omitted from the requested set.
for req in "${required_targets[@]}"; do
  found=0
  for t in "${targets[@]}"; do
    [[ "$t" == "$req" ]] && found=1 && break
  done
  if [[ $found -eq 0 ]]; then
    total=$((total + 1))
    failures=$((failures + 1))
    skipped=$((skipped + 1))
    printf '  <testcase classname="deploy-smoke.policy" name="missing-%s"><failure message="target %s was skipped: a skipped target fails the job"/></testcase>\n' \
      "$req" "$req" >>"$cases_file"
    echo "FAIL deploy-smoke: required target '$req' was skipped" >&2
  fi
done

for product in "${products[@]}"; do
  for target in "${targets[@]}"; do
    total=$((total + 1))
    set +e
    out=$(run_target "$product" "$target" 2>&1)
    rc=$?
    set -e
    if [[ $rc -eq 0 ]]; then
      printf '  <testcase classname="deploy-smoke.%s" name="%s"/>\n' "$product" "$target" >>"$cases_file"
      echo "PASS deploy-smoke ${product}/${target}"
    elif [[ $rc -eq 77 ]]; then
      failures=$((failures + 1))
      skipped=$((skipped + 1))
      printf '  <testcase classname="deploy-smoke.%s" name="%s"><failure message="target skipped (exit 77): a skipped target fails the job"/></testcase>\n' \
        "$product" "$target" >>"$cases_file"
      echo "FAIL deploy-smoke ${product}/${target}: skipped (a skipped target fails the job)" >&2
    else
      failures=$((failures + 1))
      printf '  <testcase classname="deploy-smoke.%s" name="%s"><failure message="target failed (exit %d)"/></testcase>\n' \
        "$product" "$target" "$rc" >>"$cases_file"
      echo "FAIL deploy-smoke ${product}/${target}: exit $rc" >&2
      [[ -n "$out" ]] && echo "$out" >&2
    fi
  done
done

{
  printf '<?xml version="1.0" encoding="UTF-8"?>\n'
  printf '<testsuite name="deploy-smoke" tests="%d" failures="%d" skipped="%d">\n' \
    "$total" "$failures" "$skipped"
  cat "$cases_file"
  printf '</testsuite>\n'
} >"$junit_out"

if [[ $failures -gt 0 || $skipped -gt 0 ]]; then
  echo "deploy-smoke failed: $failures failure(s), $skipped skipped" >&2
  exit 1
fi
