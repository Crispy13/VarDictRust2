# Phase 10 Post-Merge Monitor

Scope: Track the first 7 UTC days after merge for config-E2E gate health, sweep wall-clock drift, and ignored-test audit outcomes.

## Escalation Criteria

- If any monitored column fails for 2 consecutive days, open a triage issue tagged `config-e2e-regression`.
- If sweep wall-clock regresses by more than 30% versus the Phase 1 baseline, alert the owner immediately.
- If `ignore-audit.yml` finds an unexpected pass, remove the `#[ignore]` or update `scripts/ignored_tests_allowlist.txt` with justification.

| Day | Date (UTC) | ignore-audit.yml | sweep.yml wall | parity.yml push | Notes | Status |
|-----|------------|------------------|----------------|-----------------|-------|--------|
| 1 | TBD | [ ] clean | [ ] Δ <= +30% | [ ] gate green |  | [ ] OK |
| 2 | TBD | [ ] clean | [ ] Δ <= +30% | [ ] gate green |  | [ ] OK |
| 3 | TBD | [ ] clean | [ ] Δ <= +30% | [ ] gate green |  | [ ] OK |
| 4 | TBD | [ ] clean | [ ] Δ <= +30% | [ ] gate green |  | [ ] OK |
| 5 | TBD | [ ] clean | [ ] Δ <= +30% | [ ] gate green |  | [ ] OK |
| 6 | TBD | [ ] clean | [ ] Δ <= +30% | [ ] gate green |  | [ ] OK |
| 7 | TBD | [ ] clean | [ ] Δ <= +30% | [ ] gate green |  | [ ] OK |

On a successful 7-day run, summarize the monitoring results and then delete or archive this file.