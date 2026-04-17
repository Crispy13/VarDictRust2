# fuzz/ — M9 Fuzzing Infrastructure

Fuzz targets for VarDict-rs leaf utilities using `cargo-fuzz` + libfuzzer.

## Targets
- `complement_sequence` — involution on ACGT-only inputs; len preservation on arbitrary bytes.
- `substr_with_len` — length bounds for inputs the Java port contractually accepts.
- `fisher_exact` — p-values finite and in `[0, 1]` for 2x2 contingency tables.

## Prerequisites
Install cargo-fuzz once (requires nightly toolchain for the sanitizer):

```bash
cargo install cargo-fuzz
rustup install nightly
```

## Run

```bash
cd fuzz
cargo +nightly fuzz run complement_sequence -- -max_total_time=60
cargo +nightly fuzz run substr_with_len      -- -max_total_time=60
cargo +nightly fuzz run fisher_exact         -- -max_total_time=60
```

## Adding a target
1. Create `fuzz_targets/<name>.rs` with a `fuzz_target!(|data: &[u8]| { ... })`.
2. Add a `[[bin]]` section to `fuzz/Cargo.toml`.
3. Assert properties that should hold for every input (never panic, bounded outputs, etc.).

## Notes
- The fuzz crate is a standalone workspace (see `[workspace]` in `Cargo.toml`) so `cargo fuzz` manages its own lockfile.
- These are smoke fuzzers — they validate panic-freedom and invariants, not parity with Java. Parity is verified by `tests/parity_*.rs` integration tests.
