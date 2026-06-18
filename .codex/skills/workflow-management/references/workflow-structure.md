# Workflow Structure Reference

This file is the authoritative current-state inventory for the VarDict-rs workflow-management skill. Update it after every workflow infrastructure change so orientation-mode reads and change-mode reference-sync checks both use the same source of truth.

## 1. Agents

The current workflow agent set lives under `.github/agents/`. Each agent file carries the agent name, description, tools list, model, `agents:` routing list, `user-invocable`, `disable-model-invocation`, any file-level purpose note, and the full body instructions.

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

The Phase 1 workflow inventory tracks 15 current skills under `.codex/skills/*/`. Each skill file is read for its name, description, trigger contexts, which agents reference it, any agent names mentioned in the body, any file paths referenced, and its workflow phases.

### Current skill set

1. `change-impact-review`
2. `codebase-doc-manage`
3. `config-e2e-diagnosis`
4. `faithful-port`
5. `git-commit`
6. `logic-parity-audit`
7. `mismatch-repair`
8. `module-parity-test`
9. `perf-optimization`
10. `rust-freshness-verification`
11. `shard-diagnosis`
12. `tiered-config-test`
13. `workflow-inspector`
14. `workflow-management`
15. `workflow-router`

### Skill-only config E2E diagnosis path

`config-e2e-diagnosis` is the Codex entry point for active config E2E
parity diagnosis. It is intended to run as a skill-only workflow in the current
CLI session: the session follows the config E2E skill directly, writes diagnosis
and repair plan files under the current CLI session-state artifact path, asks the
user to accept those checkpoints, and invokes related skills (`mismatch-repair`,
`logic-parity-audit`, `module-parity-test`, `shard-diagnosis`, and
`change-impact-review`) directly. The `.github/agents/` files remain present for
other workflows, but this path does not require custom-agent dispatch.

## 3. Instructions

The workflow-management Phase 1 inventory currently tracks four instruction files under `.github/instructions/`. For each file, capture the `description`, `applyTo`, any file-level purpose note, and the full rule body.

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
7. `parity_e2e_sweep_somatic`

### Top-level harness files

| Binary | File | Coverage and notes |
|--------|------|--------------------|
| `parity_suite` | `tests/parity_suite.rs` | Pulls in six module-parity files: `cigar_modifier`, `cigar_parser`, `realigner`, `sam_file_parser`, `sv_processor`, and `tovars`. |
| `parity_sweep_suite` | `tests/parity_sweep_suite.rs` | Pulls in six sweep modules: `cigar_modifier_sweep`, `cigar_parser_sweep`, `realigner_sweep`, `sam_file_parser_sweep`, `sv_processor_sweep`, and `tovars_sweep`. Must run with `--test-threads=1` because it uses `GlobalReadOnlyScope::init()/clear()`. |
| `parity_e2e` | `tests/parity_e2e.rs` | Focused E2E parity harness with `parity_e2e_push` and `parity_e2e_all`. |
| `parity_config_e2e` | `tests/parity_config_e2e.rs` | Preset-driven config E2E harness. Declares `parity_config_e2e_push_*` ignored tests for each preset, plus `config_preset_alignment` and `binary_b_list_terse_format_regression`. Uses `tmp/e2e_fixtures/` goldens and `testdata/parity_regions.tsv`. |
| `parity_config_e2e_cells` | `tests/parity_config_e2e_cells.rs` | Custom `libtest-mimic` harness (`harness = false` in `Cargo.toml`) that emits ignored `parity_config_e2e_cell_<preset>_rNNN` trials and supports sharding through `VARDICT_CELL_SHARD=i/N`. |
| `parity_e2e_sweep` | `tests/parity_e2e_sweep.rs` | Custom `libtest-mimic` full-BAM E2E parity tier. Cost-gated. Uses tag-specific builders for `hg002`, `na12878_exome`, and `na12878_lowcov`, reads sweep cache from `tmp/sweep_fixtures/output/` by default, validates `manifest.json`, and supports `VARDICT_E2E_SWEEP_CONFIG`, `VARDICT_E2E_SWEEP_SHARD`, `VARDICT_E2E_SWEEP_FIXTURE_ROOT`, `VARDICT_E2E_SWEEP_BED_ROOT`, and `VARDICT_E2E_SWEEP_HEARTBEAT_LOG`. All generated chunk trials are marked ignored via `.with_ignored_flag(true)`, giving one cost-gated ignored sweep group per BAM tag. |
| `parity_e2e_sweep_somatic` | `tests/parity_e2e_sweep_somatic.rs` | Full-pair somatic sweep tier. Cost-gated. Compares Rust output against cached Java TSV shards under `tmp/sweep_fixtures/output/` by default, validates `manifest.json`, requires `--test-threads=1`, and supports `VARDICT_E2E_SWEEP_FIXTURE_ROOT`, `VARDICT_E2E_SWEEP_SOMATIC_CONFIG`, `VARDICT_E2E_SWEEP_SHARD`, `VARDICT_E2E_SWEEP_BED_ROOT`, and `CI=true`. |

### Required harness support files and directories

The inventory Phase 1 explicitly calls out these files and directories:

| Path | Notes |
|------|-------|
| `tests/parity_suite.rs` | Module-parity harness entrypoint. |
| `tests/parity_sweep_suite.rs` | Sweep harness entrypoint. |
| `tests/parity_e2e_sweep.rs` | Full-BAM sweep entrypoint. |
| `tests/parity_e2e_sweep_somatic.rs` | Somatic sweep entrypoint. |
| `tests/parity_suite/` | Module-parity test directory. |
| `tests/parity_sweep_suite/` | Module sweep test directory. |
| `tests/parity_e2e_sweep/` | Full-BAM sweep support directory. |
| `tests/parity_e2e_sweep_somatic/` | Somatic sweep support directory. |
| `tests/common/mod.rs` | Shared parity helpers, region loading, fixture lookup, Java invocation, and BAM-tag lookup. |

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

- `parity_e2e_sweep` is the full-BAM E2E parity tier. It is cost-gated, consumes cached Java TSV sweep fixtures, and builds ignored chunk trials for three BAM tags: `hg002`, `na12878_exome`, and `na12878_lowcov`.
- The tag builder files are `tests/parity_e2e_sweep/hg002_sweep.rs`, `tests/parity_e2e_sweep/na12878_exome_sweep.rs`, and `tests/parity_e2e_sweep/na12878_lowcov_sweep.rs`; each delegates to `sweep_common::build_trials(<tag>)`.
- `tests/parity_e2e_sweep/common.rs` owns cache-root discovery, manifest validation, shard parsing, chunk-plan generation, and the ignored libtest-mimic trial creation.
- `parity_e2e_sweep_somatic` is the full-pair somatic sweep tier. It is also cost-gated and currently carries one explicit ignored test, `parity_e2e_sweep_somatic_wes_il_pair`, in `tests/parity_e2e_sweep_somatic/wes_il_pair_sweep.rs`.
- `tests/parity_e2e_sweep_somatic/somatic_common.rs` validates somatic manifest entries and runs the shared tumor/normal pair logic for the `wes_il_pair` tag.

## 5. CI Workflows

Workflow-management Phase 1 tracks four CI workflows under `.github/workflows/`. For each one, note its triggers, job names, environment variables, test commands, and which test files or harnesses it runs.

### Current workflow files

| File | Triggers | Jobs and execution details |
|------|----------|----------------------------|
| `ci.yml` | `push` to `main`, `pull_request` | Job `check` (`Build + Lint + Unit Tests`) on `ubuntu-latest`. Sets `CARGO_TERM_COLOR=always`, installs `libclang-dev zlib1g-dev cmake`, exports `LIBCLANG_PATH`, installs stable Rust with `clippy` and `rustfmt`, caches `target/debug-release`, runs `cargo build --profile debug-release`, `cargo clippy --profile debug-release`, `cargo fmt -- --check`, and `cargo test --lib --profile debug-release`. This workflow does not run parity harness files. |
| `parity.yml` | `workflow_dispatch` with `module` choice input, nightly `schedule` at `0 4 * * *` | Job `parity` (`Tier 1 Parity — <module>`) runs on `self-hosted` for non-`dual_run` dispatches. Uses `VARDICT_IMPL=rust` and `VARDICT_CELL_SHARD=0/1`, gates on `scripts/check_preset_drift.sh`, `scripts/check_preset_applicability.sh`, optionally `scripts/gen_e2e_golden_tsv.sh`, optionally `scripts/config_e2e_surface_gate.sh`, then runs `cargo test` against `parity_suite`, all `parity_*` tests, `parity_e2e`, or `parity_config_e2e_cells` depending on input. Job `dual-run` runs on the nightly schedule or when `module=dual_run`, builds Rust and Java, runs `python3 scripts/dual_run.py --push-only --all-configs --verbose`, then runs `tests/parity_e2e.rs` selector `parity_e2e_push` and `tests/parity_config_e2e.rs` selector prefix `parity_config_e2e_push_`. |
| `sweep.yml` | `workflow_dispatch` with `module`, `shard_scope`, `e2e_sweep_tag`, and `e2e_sweep_somatic_tag` inputs; nightly `schedule` at `0 2 * * *` | Job `sweep` runs on `self-hosted`, times out after 180 minutes, sets `RAYON_NUM_THREADS=10` and shard-scope env, and optionally runs `scripts/config_e2e_surface_gate.sh` and `scripts/gen_e2e_golden_tsv.sh`. The main sweep step dispatches `cargo test` to `tests/parity_sweep_suite.rs`, `tests/parity_e2e.rs`, `tests/parity_config_e2e_cells.rs`, `tests/parity_e2e_sweep.rs`, and `tests/parity_e2e_sweep_somatic.rs` depending on trigger and module. Nightly mode runs all module sweep suites shard-scoped, plus `parity_e2e`, `parity_config_e2e_cells`, one `VARDICT_E2E_SWEEP_SHARD=0/4` `parity_e2e_sweep` run, and one `wes_il_pair_sweep::` somatic sweep run. |
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
9. `gen_e2e_golden_tsv.sh`
10. `gen_e2e_sweep_golden.sh`
11. `gen_somatic_sweep_bed.sh`
12. `gen_sweep_bed.sh`
13. `parity_status.sh`
14. `sample_regions.sh`
15. `sync_sweep_cache.sh`
16. `sweep_aa_check.sh`
17. `sweep_fixtures.sh`

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
