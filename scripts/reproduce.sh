#!/usr/bin/env bash
# Reproduce every dov measurement and collect the outputs under artifacts/.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release

mkdir -p artifacts
run() {
  echo "=== $* ==="
  cargo run -q --release -p dov-harness -- "$@" | tee "artifacts/$1.txt"
  echo
}

run probe
run run
run stress
run coded
run sync
run rate
run adapt
run validate
run bt

echo "All outputs collected under artifacts/"
