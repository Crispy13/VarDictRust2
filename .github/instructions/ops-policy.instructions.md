---
description: Cross-cutting operational policies for VarDict-rs work.
applyTo: '**'
---

# Operational Policies

## Build Profile
- Use `cargo build --profile debug-release` for development, debugging, and verification.
- Reserve `cargo build --release` for production or explicit release validation.

## Environment
- Activate `rust_build_env` before builds or tests, then set `LIBCLANG_PATH=$CONDA_PREFIX/lib`.
- If the conda env is broken, stop and ask whether to continue with Conda or switch to a Python `venv`.

## Temporary Files
- Create all temporary or intermediate files under `./tmp`.
- Do not use the system `/tmp` directory.

## Test Command
- Use `cargo test --profile debug-release -- --include-ignored` for validation.
- Include ignored tests because parity regressions are often parked there first.
