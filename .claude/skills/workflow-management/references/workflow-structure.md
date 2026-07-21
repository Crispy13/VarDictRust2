# Workflow Structure Reference

This file is the authoritative current-state inventory for the VarDict-rs workflow-management skill. Update it after every workflow infrastructure change so orientation-mode reads and change-mode reference-sync checks both use the same source of truth.

## 1. Agents

The current workflow agent set lives under `.claude/agents/`. Each agent file carries the agent name, description, tools list, model, `agents:` routing list, `user-invocable`, `disable-model-invocation`, any file-level purpose note, and the full body instructions.

### Current agent files

| File | Notes |
|------|-------|
| `orchestrator.agent.md` | Main routing agent for the parity workflow. |
| `cli-orchestrator.agent.md` | Codex-oriented orchestration baseline. |
| `opt-orchestrator.agent.md` | Optimization-oriented orchestration agent. |
| `module-analyst.agent.md` | Analysis agent for module-level diagnosis and planning. |
| `port-engineer.agent.md` | Implementation agent for focused code changes. |
| `parity-verifier.agent.md` | Verification agent for parity and validation steps. |
| `review-gate.agent.md` | Review and gatekeeping agent for final checks. |
| `gerneral-purpose.agent.md` | General-purpose fallback agent. |

## 2. Skills

The Phase 1 workflow inventory tracks 19 current skills under `.claude/skills/*/`. Each skill file is read for its name, description, trigger contexts, which agents reference it, any agent names mentioned in the body, any file paths referenced, and its workflow phases.

### Current skill set

1. `change-impact-review`
2. `codebase-doc-manage`
3. `config-e2e-diagnosis`
4. `faithful-port`
5. `git-commit`
6. `logic-parity-audit`
7. `mismatch-repair`
8. `module-parity-test`
9. `perf-opt-cycle`
10. `perf-optimization`
11. `production-bench`
12. `ready-fixture-folder`
13. `rust-freshness-verification`
14. `shard-diagnosis`
15. `skill-creator`
16. `tiered-config-test`
17. `workflow-inspector`
18. `workflow-management`
19. `workflow-router`

### Skill-only config E2E diagnosis path

`config-e2e-diagnosis` is the Claude Code entry point for active config E2E
parity diagnosis. It is intended to run as a skill-only workflow in the current
CLI session: the session follows the config E2E skill directly, writes diagnosis
and repair plan files under the current CLI session-state artifact path, asks the
user to accept those checkpoints, and invokes related skills (`mismatch-repair`,
`logic-parity-audit`, `module-parity-test`, `shard-diagnosis`, and
`change-impact-review`) directly. The `.claude/agents/` files remain present for
other workflows, but this path does not require subagent dispatch. `config-e2e-diagnosis`
carries its own `references/full-scope-gate-run.md` (the skill's first `references/`
file) documenting how to run Canonical Contract Step 1: the 4-tag scope table
(`hg002`, `hg005_exome`, `na12878_lowcov`, `wes_il_pair`), the two gate entry points
(`scripts/e2e_sweep_gate.sh` and `scripts/full_gate_tag.sh`), the env matrix, the
CM-PILEUP streaming lever, and the real-green verification checklist.

## 3. Instructions

The workflow-management Phase 1 inventory currently tracks four instruction files under `.claude/instructions/`. For each file, capture the `description`, `applyTo`, any file-level purpose note, and the full rule body.

### Current instruction files

| File | applyTo | Notes |
|------|---------|-------|
| `ops-policy.instructions.md` | `**` | Cross-cutting operational policy file. |
| `rust-parity.instructions.md` | `**/*.rs` | Rust parity rules for faithful Java-to-Rust behavior. |
| `rust.instructions.md` | `**/*.rs` | General Rust coding conventions. |
| `terminal-reconciliation.instructions.md` | `**` | Workaround for avoiding being stuck after terminal finishes. |

## 4. Test Harness

The workflow test harness spans the `tests/` tree and is split into module parity, module sweep parity, focused E2E/config parity, and full-BAM/full-pair sweep tiers. Phase 1 should note module names, fixture paths, test function names, `#[ignore]` annotations, and which modules each harness covers.

### Parity harness binaries

The current parity harness binaries are:

1. `parity_suite`
2. `parity_sweep_suite`
3. `parity_e2e`
4. `parity_config_e2e`
5. `parity_config_e2e_cells`
6. `parity_e2e_sweep`
7. `parity_fuzz`

### Top-level harness files

| Binary | File | Coverage and notes |
|--------|------|--------------------|
| `parity_suite` | `tests/parity_suite.rs` | Pulls in six module-parity files: `cigar_modifier`, `cigar_parser`, `realigner`, `sam_file_parser`, `sv_processor`, and `tovars`. |
| `parity_sweep_suite` | `tests/parity_sweep_suite.rs` | Pulls in six sweep modules: `cigar_modifier_sweep`, `cigar_parser_sweep`, `realigner_sweep`, `sam_file_parser_sweep`, `sv_processor_sweep`, and `tovars_sweep`. Must run with `--test-threads=1` because it uses `GlobalReadOnlyScope::init()/clear()`. |
| `parity_e2e` | `tests/parity_e2e.rs` | Focused E2E parity harness with `parity_e2e_push` and `parity_e2e_all`. |
| `parity_config_e2e` | `tests/parity_config_e2e.rs` | Preset-driven config E2E harness. Declares `parity_config_e2e_push_*` ignored tests for each preset, plus `config_preset_alignment` and `binary_b_list_terse_format_regression`. Uses `tmp/e2e_fixtures/` goldens and `testdata/parity_regions.tsv`. |
| `parity_config_e2e_cells` | `tests/parity_config_e2e_cells.rs` | Custom `libtest-mimic` harness (`harness = false` in `Cargo.toml`) that emits ignored `parity_config_e2e_cell_<preset>_rNNN` trials and supports sharding through `VARDICT_CELL_SHARD=i/N`. |
| `parity_e2e_sweep` | `tests/parity_e2e_sweep.rs` | Custom `libtest-mimic` full-BAM E2E parity tier. Cost-gated. Uses tag-specific builders for `hg002`, `na12878_exome`, `na12878_lowcov`, and the somatic tumor/normal pair tag `wes_il_pair`, reads sweep cache from `tmp/sweep_fixtures/output/` by default, validates `manifest.json`, and supports `VARDICT_E2E_SWEEP_CONFIG`, `VARDICT_E2E_SWEEP_SHARD`, `VARDICT_E2E_SWEEP_FIXTURE_ROOT`, `VARDICT_E2E_SWEEP_BED_ROOT`, and `VARDICT_E2E_SWEEP_HEARTBEAT_LOG`. All generated chunk trials are marked ignored via `.with_ignored_flag(true)`, giving one cost-gated ignored sweep group per BAM tag (somatic `wes_il_pair` trials are filtered via `wes_il_pair_sweep::` and require `--test-threads=1`). |
| `parity_fuzz` | `tests/parity_fuzz.rs` | Differential parity **fuzzer** (germline + somatic). proptest generates synthetic single-contig genomes (`tests/parity_fuzz/generator.rs`), materializes ref+sorted/indexed BAM(s) via `samtools` (`tests/parity_fuzz/synth.rs`), runs BOTH `vardict_rs` and VarDictJava over one `-R` region, and asserts byte-identical output after sort-normalization (`tests/parity_fuzz/oracle.rs`). Each locus independently exercises an SNV/deletion/insertion/MNV, optionally with clips, flag-filtered noise reads, region-boundary cropping, a minority of **quality-noise reads** (straddling MAPQ / base-quality floors), and a minority of loci emitted as **overlapping proper read-pairs** (mate1 fwd / mate2 rev overlapping across the variant, exercising VarDict's paired-read path). **Five** proptest tests (case count via `PARITY_FUZZ_CASES`, default 64): `pbt_germline_snv_parity` (default preset), `pbt_germline_preset_parity` (each genome re-run under a curated germline **config preset** via `generator::arb_germline_preset`, flags passed through `oracle`'s `extra_flags`), the **somatic paired T/N** lane — `pbt_somatic_parity` and `pbt_somatic_preset_parity` — which builds two BAMs (tumor + normal, differing only in per-locus alt fraction so the `VarLabel` status spans StrongSomatic/LikelySomatic/AFDiff/Germline/LikelyLOH) from one shared reference via `synth::materialize_paired`, runs both tools paired (`-b "tumor|normal"` via `oracle::run_*_paired`), and compares the 55/61-col somatic TSV under a curated somatic preset set (`generator::arb_somatic_preset`, incl. somatic-specific `-V`/`-I`); plus the **unique-mode overlap-dedup** lane — `pbt_unique_mode_paired_parity` — which runs all-proper-pair genomes (`generator::arb_paired_genome`, no unpaired reads) under `-u`/`--UN` (`generator::arb_unique_mode_preset`), the only flags that trigger `skipOverlappingReads` (count a pair's overlap once); a minority of fragments carry **disagreeing mates** (different alleles) so the mate the mode skips decides the counted allele. Plus loop-proof + strategy-sampling unit tests. This is the **first live paired VDJ↔VDR differential runner** (somatic was previously only checked against cached goldens). The five `pbt_*` tests are **`#[ignore]`-gated** (opt in with `-- --include-ignored`; run nightly by `parity.yml`'s `fuzz` job) and require the `vdr` conda env (samtools) + a built `target/debug-release/vardict_rs` (override with `VARDICT_RS_BIN`); VDJ pinned by the submodule at `4e362c0`. Corpus of found divergences persists in `tests/parity_fuzz.proptest-regressions`. |

### Required harness support files and directories

The inventory Phase 1 explicitly calls out these files and directories:

| Path | Notes |
|------|-------|
| `tests/parity_suite.rs` | Module-parity harness entrypoint. |
| `tests/parity_sweep_suite.rs` | Sweep harness entrypoint. |
| `tests/parity_e2e_sweep.rs` | Full-BAM sweep entrypoint (also covers the somatic `wes_il_pair` pair tag). |
| `tests/parity_suite/` | Module-parity test directory. |
| `tests/parity_sweep_suite/` | Module sweep test directory. |
| `tests/parity_e2e_sweep/` | Full-BAM sweep support directory (includes `wes_il_pair_sweep.rs` for the somatic pair tag). |
| `tests/common/mod.rs` | Shared parity helpers, region loading, fixture lookup, Java invocation, and BAM-tag lookup. |
| `tests/parity_fuzz.rs` | Differential parity fuzzer entrypoint (germline). |
| `tests/parity_fuzz/` | Fuzzer support directory: `generator.rs` (proptest strategies), `synth.rs` (samtools BAM/FASTA synthesis), `oracle.rs` (run-both + normalize + compare). |

### Module parity coverage

The `tests/parity_suite/` directory contains one file per module under test:

| File | Module coverage | Named test functions | Fixture notes |
|------|-----------------|----------------------|---------------|
| `tests/parity_suite/cigar_modifier.rs` | `cigar_modifier` | `parity_cigar_modifier_all_regions` | Uses shared parity helpers rooted in `testdata/parity_regions.tsv` and `testdata/fixtures/`. |
| `tests/parity_suite/cigar_parser.rs` | `cigar_parser` | `parity_cigar_parser_all_regions`, `parity_cigar_parser_config_t1_02_10_116065606_116065839` | Uses shared region config and module fixtures under `testdata/fixtures/`. |
| `tests/parity_suite/realigner.rs` | `realigner` | `parity_realigner_all_regions`, `parity_realigner_region_1_2324084_2324612`, `parity_realigner_region_1_9967324_9968024`, `parity_realigner_region_1_8926126_8926826`, `parity_realigner_config_t1_01_1_155006164_155006864` | Uses shared region config and module fixtures under `testdata/fixtures/`. |
| `tests/parity_suite/sam_file_parser.rs` | `sam_file_parser` | `parity_sam_file_parser_all_regions` | Uses shared region config and module fixtures under `testdata/fixtures/`. |
| `tests/parity_suite/sv_processor.rs` | `sv_processor` | `parity_sv_processor_all_regions`, `parity_sv_processor_region_1_9967324_9968024`, `parity_sv_processor_config_t1_01_14_106517915_106518615`, `parity_sv_processor_config_t1_01_8_20002977_20003677` | Uses shared region config and module fixtures under `testdata/fixtures/`. |
| `tests/parity_suite/tovars.rs` | `tovars` | `parity_tovars_all_regions` | Uses shared region config and module fixtures under `testdata/fixtures/`. |

### Sweep module coverage

The `tests/parity_sweep_suite/` directory contains one full-sweep parity file per module. Every sweep test is currently `#[ignore]`d as a cost-gated Sweep-tier test and uses `tmp/sweep_fixtures` by default unless `VARDICT_SWEEP_FIXTURE_DIR` overrides it.

| File | Module coverage | Ignored test function |
|------|-----------------|-----------------------|
| `tests/parity_sweep_suite/cigar_modifier_sweep.rs` | `cigar_modifier` | `parity_cigar_modifier_sweep` |
| `tests/parity_sweep_suite/cigar_parser_sweep.rs` | `cigar_parser` | `parity_cigar_parser_sweep` |
| `tests/parity_sweep_suite/realigner_sweep.rs` | `realigner` | `parity_realigner_sweep` |
| `tests/parity_sweep_suite/sam_file_parser_sweep.rs` | `sam_file_parser` | `parity_sam_file_parser_sweep` |
| `tests/parity_sweep_suite/sv_processor_sweep.rs` | `sv_processor` | `parity_sv_processor_sweep` |
| `tests/parity_sweep_suite/tovars_sweep.rs` | `tovars` | `parity_tovars_sweep` |

### Full-BAM and somatic sweep notes

- `parity_e2e_sweep` is the full-BAM E2E parity tier. It is cost-gated, consumes cached Java TSV sweep fixtures, and builds ignored chunk trials for three single-sample BAM tags (`hg002`, `na12878_exome`, `na12878_lowcov`) plus the somatic tumor/normal pair tag `wes_il_pair`. It is somatic-aware via an internal `SweepMode { Single, Somatic }` and runs the somatic pair as parallel, thread-local chunk trials (same `--test-threads=N` parallelism as the single-sample tags).
- The tag builder files are `tests/parity_e2e_sweep/hg002_sweep.rs`, `tests/parity_e2e_sweep/na12878_exome_sweep.rs`, `tests/parity_e2e_sweep/na12878_lowcov_sweep.rs`, and `tests/parity_e2e_sweep/wes_il_pair_sweep.rs`; each delegates to `sweep_common::build_trials(<tag>)`.
- `tests/parity_e2e_sweep/common.rs` owns cache-root discovery, manifest validation (single key `{config}:{tag}` and somatic key `{config}:somatic:{tag}`), shard parsing, chunk-plan generation, the `SweepMode` classification in `prepare_tag_context`, the per-mode `SimpleMode`/`SomaticMode` chunk run, and the ignored libtest-mimic trial creation.
- The standalone `parity_e2e_sweep_somatic` binary has been retired; somatic `wes_il_pair` sweep parity now runs entirely through the `parity_e2e_sweep` germline harness, filtered by the `wes_il_pair_sweep::` trial prefix (e.g. `cargo test --profile debug-release --test parity_e2e_sweep wes_il_pair_sweep:: -- --include-ignored --test-threads=1`).

## 5. CI Workflows

Workflow-management Phase 1 tracks four CI workflows under `.claude/workflows/`. For each one, note its triggers, job names, environment variables, test commands, and which test files or harnesses it runs.

### Current workflow files

| File | Triggers | Jobs and execution details |
|------|----------|----------------------------|
| `ci.yml` | `push` to `main`, `pull_request` | Job `check` (`Build + Lint + Unit Tests`) on `ubuntu-latest`. Sets `CARGO_TERM_COLOR=always`, installs `libclang-dev zlib1g-dev cmake`, exports `LIBCLANG_PATH`, installs stable Rust with `clippy` and `rustfmt`, caches `target/debug-release`, runs `cargo build --profile debug-release`, `cargo clippy --profile debug-release`, `cargo fmt -- --check`, and `cargo test --lib --profile debug-release`. This workflow does not run parity harness files. |
| `parity.yml` | `workflow_dispatch` with `module` choice input, nightly `schedule` at `0 4 * * *` | Job `parity` (`Tier 1 Parity — <module>`) runs on `self-hosted` for non-`dual_run` dispatches. Uses `VARDICT_IMPL=rust` and `VARDICT_CELL_SHARD=0/1`, gates on `scripts/check_preset_drift.sh`, `scripts/check_preset_applicability.sh`, optionally `scripts/gen_e2e_golden_tsv.sh`, optionally `scripts/config_e2e_surface_gate.sh`, then runs `cargo test` against `parity_suite`, all `parity_*` tests, `parity_e2e`, or `parity_config_e2e_cells` depending on input. Job `dual-run` runs on the nightly schedule or when `module=dual_run`, builds Rust and Java, runs `python3 scripts/dual_run.py --push-only --all-configs --verbose`, then runs `tests/parity_e2e.rs` selector `parity_e2e_push` and `tests/parity_config_e2e.rs` selector prefix `parity_config_e2e_push_`. |
| `sweep.yml` | `workflow_dispatch` with `module`, `shard_scope`, `e2e_sweep_tag`, and `e2e_sweep_somatic_tag` inputs; nightly `schedule` at `0 2 * * *` | Job `sweep` runs on `self-hosted`, times out after 180 minutes, sets `RAYON_NUM_THREADS=10` and shard-scope env, and optionally runs `scripts/config_e2e_surface_gate.sh` and `scripts/gen_e2e_golden_tsv.sh`. The main sweep step dispatches `cargo test` to `tests/parity_sweep_suite.rs`, `tests/parity_e2e.rs`, `tests/parity_config_e2e_cells.rs`, and `tests/parity_e2e_sweep.rs` depending on trigger and module; the somatic sweep run also targets `tests/parity_e2e_sweep.rs`, filtered by the `wes_il_pair_sweep::` (or `${{ inputs.e2e_sweep_somatic_tag }}_sweep::`) trial prefix. Nightly mode runs all module sweep suites shard-scoped, plus `parity_e2e`, `parity_config_e2e_cells`, one `VARDICT_E2E_SWEEP_SHARD=0/4` `parity_e2e_sweep` run, and one `wes_il_pair_sweep::` somatic sweep run. |
| `ignore-audit.yml` | `workflow_dispatch`, nightly `schedule` at `30 3 * * *` | Job `ignore-audit` (`Audit Ignored Tests`) on `self-hosted`. Sets `VARDICT_IMPL=rust`, runs `cargo build --profile debug-release`, then `bash scripts/check_ignored_tests.sh`. This workflow audits the ignored-tests policy rather than running a named parity harness directly. |

## 6. Scripts

Workflow-management Phase 1 tracks parity-related shell scripts, Python scripts, library helpers, and the ignored-tests allowlist under `scripts/`. The list below is the current inventory referenced by the skill.

### Shell scripts

1. `aa_gate.sh`
2. `batch_fixtures.sh`
3. `bisect_parity.sh`
4. `check_ignored_tests.sh`
5. `check_preset_applicability.sh`
6. `check_preset_drift.sh`
7. `config_e2e_surface_gate.sh`
8. `e2e_sweep_gate.sh`
9. `full_gate_tag.sh`
10. `gen_e2e_golden_tsv.sh`
11. `gen_e2e_sweep_golden.sh`
12. `gen_somatic_sweep_bed.sh`
13. `gen_sweep_bed.sh`
14. `parity_status.sh`
15. `sample_regions.sh`
16. `sync_sweep_cache.sh`
17. `sweep_aa_check.sh`
18. `sweep_fixtures.sh`

### Python scripts

1. `backfill_chunks_json.py`
2. `dual_run.py`
3. `e2e_sweep_gate.py`
4. `io_backend_bench.py`
5. `pilot_generate.py`
6. `sample_regions.py`
7. `sort_sweep_fixtures.py`
8. `sweep_fixtures_chunk_parallel.py`
9. `sweep_fixtures_parallel.py`
10. `sweep_generate_v2.py`

### Library helpers and policy files

| Path | Notes |
|------|-------|
| `lib/merge_manifest.py` | Shared Python helper module under `scripts/lib/`. |
| `scripts/ignored_tests_allowlist.txt` | Allowlist for ignored tests expected to remain ignored. |

## 7. Build Configuration

Workflow-management Phase 1 reads `Cargo.toml` for workflow-relevant test configuration, especially `[dev-dependencies]`, `[profile.debug-release]`, and any `[[test]]` or `[[bench]]` sections.

### Current workflow-relevant `Cargo.toml` sections

- `[dev-dependencies]` currently includes `criterion`, `insta`, `libtest-mimic`, `proptest`, `sha2`, and `zstd`.
- `[profile.debug-release]` inherits from `release` and sets `debug = true`.
- `[[test]] name = "parity_config_e2e_cells"` sets `harness = false`.
- `[[test]] name = "parity_e2e_sweep"` sets `harness = false`.
- No `[[bench]]` sections are currently present in `Cargo.toml`.

## 8. Sweep Runtime Resources & Fixture Generation

Where the sweep caches and BEDs live (canonical hdd cache dirs, the already-generated sibling-repo
BEDs, per-tag `bed_sha256` provenance, the duplicate-cache trap) **and the additive-safe start-here
runbook for generating e2e parity fixtures** are documented in a dedicated reference:
[`sweep-fixture-generation.md`](sweep-fixture-generation.md). Any agent doing sweep generation or
sweep parity must read it first; the generator `scripts/gen_e2e_sweep_golden.sh` header also points there.
