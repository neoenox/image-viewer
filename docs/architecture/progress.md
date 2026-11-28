# Architecture refactor progress

Work is performed in the isolated worktree `C:/tmp/iv-arch` on branch
`arch/systematic-refactor`, based on `origin/main` at v0.2.12.

## Current phase

Complete. Independent review found no product regressions; its two guard
findings were fixed (registry guard repointed to the shared validator, release
verification limitations documented with a manual dispatch path). Remaining
candidates are explicitly deferred in the task pool rather than partially
implemented.

## Evidence

- Baseline: `C:/tmp/iv-arch-baseline.log`
- Round 1: `C:/tmp/iv-arch-batch1.log`, `batch2b.log`, `batch3.log`
- Round 2: `C:/tmp/iv-arch-round2b.log`
- Round 3: `C:/tmp/iv-arch-assoc.log`, release guard validated against v0.2.12 assets
- Final: `C:/tmp/iv-arch-final2.log`; registry guard PASS; workflow YAML parsed

## Rounds

| Round | Goal | State | Evidence |
|---|---|---|---|
| 0 | Capture baseline and architecture map | done | baseline.log, architecture-map.md |
| 1 | Remove duplication and establish single sources of truth | done | batch1/batch2b/batch3 logs |
| 2 | Tighten module boundaries and state ownership | done | round2b.log; R2-02 blocked |
| 3 | Add enforceable architecture guards | done | assoc.log, release guard validation |
| 4 | Independent review, full verification, batch commits | done | final2.log, review findings closed |

## Rules for this program

- One logical change per batch, migrated callers included, old implementation removed in the same batch.
- No behavior, data-format, CLI, env-var, or external-contract changes unless explicitly approved.
- Every batch ends with focused tests, then the full suite, clippy and fmt.
- Deletion test: keep an abstraction only if removing it would spread complexity back to callers.
