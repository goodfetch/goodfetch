#!/usr/bin/env bash
# Prints the run's variables that can change what the gate's steps and
# `just quick`'s run or how, as sorted `NAME=value` lines: they join the host
# fingerprint (`xtask gate`'s `host`), so a result counts only for a run in
# the same environment. `xtask gate` leaves out the launchers' own
# (GOODFETCH_GATE_MIN_FREE_GB, GOODFETCH_GATE_RECORDS, GOODFETCH_GATE_LAYER,
# GOODFETCH_QUICK_BASE) and the thread and job caps the gate removes before
# its steps.
set -euo pipefail

env | grep -E '^(CI|GOODFETCH_[A-Z0-9_]+|NEXTEST_[A-Z0-9_]+|CARGO_[A-Z0-9_]+|RUST[A-Z0-9_]*|UV_[A-Z0-9_]+|NODE_[A-Z0-9_]+)=' | LC_ALL=C sort || true
