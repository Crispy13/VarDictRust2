#!/usr/bin/env bash
set -eo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

# shellcheck disable=SC1091
source "$(conda info --base)/etc/profile.d/conda.sh"
set +e
conda activate rust_build_env
conda_status=$?
set -e
if [[ "$conda_status" -ne 0 && "${CONDA_DEFAULT_ENV:-}" != "rust_build_env" ]]; then
	exit "$conda_status"
fi
set -u
export LIBCLANG_PATH="${CONDA_PREFIX}/lib"

exec python3 -m scripts.e2e_sweep_gate "$@"