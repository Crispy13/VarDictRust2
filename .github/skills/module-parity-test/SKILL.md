---
name: module-parity-test
description: "Run module-level parity tests comparing Rust output against Java golden JSONL fixtures. Use when: verifying a ported module, running parity tests, checking module output, generating golden fixtures, running A-A gate, module parity check, JSONL fixture comparison, after porting a pipeline module. Always use this skill after faithful-port completes a pipeline module — it is the verification gate. Do NOT use for fixing mismatches (use mismatch-repair) or diagnosing specific shard failures (use shard-diagnosis)."
argument-hint: "Module name, e.g. 'cigar_parser' or 'realigner' or 'all'"
---

# Module Parity Test

Verify that a Rust module produces byte-identical output to Java by comparing against golden JSONL fixtures. This skill covers the full cycle: fixture generation, determinism verification (A-A gate), Rust parity comparison, and failure routing.

## When This Gets Called

The `faithful-port` skill hands off here after Phase 3 (structural review) for pipeline modules. The porting agent invokes this skill with a module name. This skill runs the verification, then either confirms parity or routes failures to `shard-diagnosis`.

```
faithful-port → module-parity-test → PASS (advance module)
                                   → FAIL → shard-diagnosis → mismatch-repair
```

## Prerequisites

Before running parity tests, verify these are in place:

1. **Java parity writers exist** in `VarDictJava/src/main/java/com/astrazeneca/vardict/parity/`. If not, they need to be written first — see [JSONL format contract](./references/jsonl-format-contract.md).
2. **Rust module is implemented** and compiles: `cargo build --profile debug-release`
3. **Rust data types have `#[derive(serde::Serialize)]`** on all types used in the module's output scope data.
4. **`testdata/parity_regions.tsv`** exists with sampled regions (100 regions across 3 BAMs).

## Module Names

| Module | Rust Test File | Java Writer | Fixture Dir |
|--------|---------------|-------------|-------------|
| `cigar_parser` | `tests/parity_cigar_parser.rs` | `CigarParserJsonl.java` | `testdata/fixtures/cigar_parser/` |
| `realigner` | `tests/parity_realigner.rs` | `RealignerJsonl.java` | `testdata/fixtures/realigner/` |
| `sv_processor` | `tests/parity_sv_processor.rs` | `SVProcessorJsonl.java` | `testdata/fixtures/sv_processor/` |
| `tovars` | `tests/parity_tovars.rs` | `ToVarsBuilderJsonl.java` | `testdata/fixtures/tovars/` |

## Procedure

### Step 1: Check Golden Fixtures

Check whether golden fixtures already exist for the target module:

```bash
ls testdata/fixtures/<module>/*.jsonl 2>/dev/null | wc -l
```

- **If fixtures exist and are current:** skip to Step 3.
- **If fixtures don't exist or Java writers were modified:** proceed to Step 2.

"Current" means the Java parity writer hasn't changed since the fixtures were generated. Check via:
```bash
git log -1 --format='%h %ci' -- VarDictJava/src/main/java/com/astrazeneca/vardict/parity/
```

### Step 2: Generate Golden Fixtures

#### 2a: A-A Gate (Determinism Check)

Run VarDictJava twice on all regions and diff outputs. This catches HashMap iteration order, float formatting drift, and other nondeterminism.

```bash
scripts/batch_fixtures.sh --mode aa
```

**Expected output:** `PASS` for every region. If any region fails the A-A gate:
- Check the diff file in `tmp/batch_fixtures/aa/diffs/`
- The Java writer has a nondeterminism bug — fix the writer before proceeding
- Common causes: unsorted HashMap serialization, `HashSet` iteration order, multi-threaded access

Do NOT proceed to fixture generation until the A-A gate passes 100%.

#### 2b: Generate Fixtures

```bash
scripts/batch_fixtures.sh --mode generate
```

This runs VarDictJava once per region and copies JSONL output to `testdata/fixtures/<module>/`.

**Verify:** each fixture file has exactly 2 lines (meta + data):
```bash
for f in testdata/fixtures/<module>/*.jsonl; do
  lines=$(wc -l < "$f")
  [[ "$lines" -eq 2 ]] || echo "BAD: $f has $lines lines"
done
```

### Step 3: Run Rust Parity Tests

#### 3a: Single Module

```bash
cargo test --profile debug-release --test parity_<module> -- --include-ignored
```

The `--include-ignored` flag is required because parity tests start as `#[ignore]` and get un-ignored as modules are implemented.

#### 3b: All Modules

```bash
cargo test --profile debug-release --test 'parity_*' -- --include-ignored
```

### Step 4: Interpret Results

#### All tests pass → PARITY ACHIEVED

Report success. The module's parity test `#[ignore]` attribute should be removed to make it part of the regular test suite. Update the active plan marking the module as parity-verified.

#### Tests fail → Route to Diagnosis

For each failing test:

1. **Read the failure output.** The `assert_module_parity()` function reports:
   - Byte position of first divergence
   - 80-char window showing Rust vs Java around the divergence point

2. **If the failure is structural** (missing field, wrong field order, wrong wrapper):
   - Check that `#[serde(rename = "javaFieldName")]` is correct for every field
   - Check that map serialization uses sorted keys (custom `serialize_with`)
   - Check that `VariationMap` uses the `{entries: [...], sv: ...}` wrapper format

3. **If the failure is a value mismatch** (correct structure, wrong number/string):
   - Hand off to `shard-diagnosis` with the region and module name
   - The diagnosis skill will identify the first divergent field
   - Then `mismatch-repair` fixes the Rust logic

4. **If the failure is a float formatting difference:**
   - Check whether the Rust code uses `format!("{:.N}", v)` instead of the project's `java_format_double` helper
   - Java `Double.toString()` and Rust `serde_json` (Ryu) should match for standard values, but `DecimalFormat` formatted values need the HALF_UP helper

### Step 5: Iterate

After `mismatch-repair` fixes a divergence, re-run the failing test:

```bash
cargo test --profile debug-release --test parity_<module> -- --include-ignored <test_name>
```

Repeat Steps 4-5 until all parity tests pass.

## Test Architecture

The module parity tests form Layer 2 of a three-layer test pyramid:

```
┌──────────────────────────────────────┐
│ Layer 3: End-to-End TSV Diff         │ ← Full pipeline. Final gate.
├──────────────────────────────────────┤
│ Layer 2: Module JSONL Parity         │ ← THIS SKILL. Per-module golden.
├──────────────────────────────────────┤
│ Layer 1: Unit + Property Tests       │ ← Fast. Synthetic data.
└──────────────────────────────────────┘
```

### Chained Testing

After module N achieves parity, module N+1 can use N's golden output as input. This isolates failures to a single module:

```
CigarParser golden → Realigner input → compare Realigner output vs Realigner golden
```

The test for module N+1 loads module N's golden JSONL, deserializes it as Rust types, runs module N+1, and compares against module N+1's golden.

## Existing Infrastructure

| File | Purpose |
|------|---------|
| `scripts/batch_fixtures.sh` | A-A gate + fixture generation for all modules/regions |
| `scripts/aa_gate.sh` | Single-region A-A gate |
| `scripts/sample_regions.py` | Sample coverage-stratified regions from a BAM |
| `testdata/parity_regions.tsv` | 100 sampled regions (3 BAMs × ~33 regions each) |
| `tests/common/mod.rs` | Shared helpers: `load_region_config()`, `load_golden_data()`, `golden_fixture_path()` |
| `tests/parity_cigar_parser.rs` | CigarParser parity test (ignored until module is ported) |
| `tests/parity_realigner.rs` | Realigner parity test (ignored) |
| `tests/parity_sv_processor.rs` | SVProcessor parity test (ignored) |
| `tests/parity_tovars.rs` | ToVars parity test (ignored) |

## JSONL Format

See [JSONL format contract](./references/jsonl-format-contract.md) for the canonical spec covering file structure, type mappings, float format, and field naming conventions. Both Java writers and Rust serde output must produce bytes matching this contract.

## Relationship to Other Skills

- **faithful-port** (upstream): Hands off here after porting a pipeline module.
- **shard-diagnosis** (downstream): Diagnoses specific parity failures identified by this skill.
- **mismatch-repair** (downstream): Fixes Rust logic divergences found by shard-diagnosis.
- **tiered-config-test** (further downstream): Expands config coverage after parity is achieved.
- **codebase-doc-manage** (parallel): Maintains Java module docs consumed during porting.
