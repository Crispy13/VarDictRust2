# Operational Policies

## Build Profile
- Use `cargo build --profile debug-release` for development, debugging, and verification.
- Reserve `cargo build --release` for production or explicit release validation.

## Environment
- Activate `vdr` before builds or tests, then set `LIBCLANG_PATH=$CONDA_PREFIX/lib`.
- If the conda env is broken, stop and ask whether to continue with Conda or switch to a Python `venv`.

## Temporary Files
- Create all temporary or intermediate files under `./tmp`.
- Do not use the system `/tmp` directory.

## Test Command
- Use `cargo test --profile debug-release -- --include-ignored` for validation.
- Include ignored tests because parity regressions are often parked there first.

## Ignored Tests
- Every `#[ignore]` must have a message explaining: why it's ignored, how to run it, and what prerequisite data/workflow it depends on.
- Categories: `Sweep` (cost-gated, run via sweep.yml), `Nightly` (E2E all, run via sweep.yml or parity.yml), `Temporary` (remove when blocker resolved).
- Allowlist: `scripts/ignored_tests_allowlist.txt` — tests expected to remain ignored. Update when adding/removing `#[ignore]`.
- Audit: `.github/workflows/ignore-audit.yml` runs nightly at 03:30 UTC. Flags any passing ignored test not in the allowlist.
- When a previously-failing ignored test starts passing, either remove `#[ignore]` or add to allowlist with justification.

## Terminal Usage
- never emit exit at the end of a sync terminal command. It causes agents to wait forever even after the command completes.
