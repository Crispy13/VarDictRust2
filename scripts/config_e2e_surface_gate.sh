#!/usr/bin/env bash
set -euo pipefail
export VARDICT_IMPL="${VARDICT_IMPL:-rust}"
unset PARITY_REGION_INDEX
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"
mkdir -p tmp

# Binary A — push count + aggregator count
A_LIST="tmp/surface-gate-a-list.txt"
cargo test --profile debug-release --test parity_config_e2e -- --list --format=pretty > "$A_LIST"
push_count=$(grep -cE '^parity_config_e2e_push_[a-z0-9_]+: test$' "$A_LIST" || true)
all_count=$(grep -cE '^parity_config_e2e_all_[a-z0-9_]+: test$' "$A_LIST" || true)
[[ "$push_count" == "44" ]] || { echo "FAIL: push=$push_count expected 44"; exit 1; }
[[ "$all_count"  == "0"  ]] || { echo "FAIL: aggregator=$all_count expected 0"; exit 1; }

# Binary B — cell count
B_LIST="tmp/surface-gate-b-list.txt"
cargo test --profile debug-release --test parity_config_e2e_cells -- --list --format=terse > "$B_LIST"
cell_count=$(grep -cE '^parity_config_e2e_cell_[a-z0-9_]+_r[0-9]{3}: test$' "$B_LIST" || true)
[[ "$cell_count" == "4400" ]] || { echo "FAIL: cell=$cell_count expected 4400"; exit 1; }

# Ignore-attr contract
B_IGNORED="tmp/surface-gate-b-ignored.txt"
cargo test --profile debug-release --test parity_config_e2e_cells -- --list --ignored --format=terse > "$B_IGNORED"
ignored_cell_count=$(grep -cE '^parity_config_e2e_cell_[a-z0-9_]+_r[0-9]{3}: test$' "$B_IGNORED" || true)
[[ "$ignored_cell_count" == "4400" ]] || { echo "FAIL: ignored=$ignored_cell_count expected 4400"; exit 1; }
[[ "$cell_count" == "$ignored_cell_count" ]] || { echo "FAIL: not all cells ignored"; exit 1; }

# Source-level ignore-flag contract
if ! grep -Fq '.with_ignored_flag(true)' tests/parity_config_e2e_cells.rs 2>/dev/null; then
    echo "FAIL: .with_ignored_flag(true) missing from tests/parity_config_e2e_cells.rs"; exit 1
fi

# libtest-mimic version pin sanity
if ! grep -Eq 'libtest-mimic\s*=\s*"0\.8' Cargo.toml 2>/dev/null; then
    echo "WARN: libtest-mimic version pin not at 0.8.x — verify format regression test still passes"
fi

# Exact-equality gate on expected-failure set
BASELINE="testdata/expected_failing_cells.txt"
[[ -f "$BASELINE" ]] || { echo "FAIL: missing baseline $BASELINE"; exit 1; }
set +e
cargo test --profile debug-release --test parity_config_e2e_cells -- --include-ignored --test-threads=10 2>&1 | tee tmp/surface-gate-run.log
set -e
{ grep -E '^test parity_config_e2e_cell_.* FAILED$' tmp/surface-gate-run.log || true; } | awk '{print $2}' | sort > tmp/surface-gate-fails.txt
diff -u <(sort "$BASELINE") tmp/surface-gate-fails.txt || { echo "FAIL: failing set diverged"; exit 1; }

echo "OK: Binary A 44 push / 0 agg; Binary B 4400 cells; ignore contract verified; failing set matches baseline."