# Architecture optimization task pool

Statuses: `discovered`, `ready`, `in progress`, `done`, `blocked`.

## Round 1 — single source of truth and external-contract guards

| ID | Area | Task | Value | Risk | Status |
|---|---|---|---|---|---|
| R1-01 | Settings paths | Add `settings::data_dir()` and migrate `report`, `update`, `app` derivations | high | low | done |
| R1-02 | CLI | Add one CLI parser used by `main` and `app::run`; keep all existing flags | high | low | done |
| R1-03 | Installer contract | Remove stale video text/extensions from Inno script; align with `SUPPORTED_EXTS` | high | low | done |
| R1-04 | Enforced guard | CI guard for no-video invariant and `.iss`/`SUPPORTED_EXTS` parity | high | low | done |
| R1-05 | Docs | Fix README coverage/flags/assets drift found in mapping | medium | low | done |

## Round 2 — boundaries and cohesion

| ID | Area | Task | Value | Risk | Status |
|---|---|---|---|---|---|
| R2-01 | UI/state | Move shared status/modal helpers into a small `viewer_state` module only if deletion test passes | medium | medium | discovered |
| R2-02 | Rendering | Move detail-region request decisions out of `view` into `app` | high | medium | blocked |
| R2-03 | Worker seams | Wrap loader constants/keys behind stable app-facing methods | medium | medium | discovered |
| R2-04 | Error handling | Unify `upscale` panic/error path with `sync::catch_panic` | medium | low | done |
| R2-05 | Tests | Consolidate duplicated settings test helpers without losing cases | low | low | done |

## Round 3 — enforceable architecture

| ID | Area | Task | Value | Risk | Status |
|---|---|---|---|---|---|
| R3-01 | Contracts | Golden settings-file contract test | medium | low | done |
| R3-02 | Release | Verify release completeness and checksum reverse coverage | high | medium | done |
| R3-03 | Assoc parity | Single-source assoc constants/validation for both cfg variants | high | medium | done |
| R3-04 | Docs guard | Check README numbers/asset names against workflow constants | medium | low | discovered |
| R3-05 | CLI contract | Smoke test for `--check-update` and `--print-assoc-config` flag positions | medium | low | discovered |

## Blocked / awaiting confirmation

- R3-04 is deferred: a regex doc guard would be more brittle than the value it protects.
- R3-05 is covered by `cli.rs` unit tests; subprocess smoke adds no new contract coverage.
- R2-02 needs a redesigned return contract: the current detail branch uses an early `return` from the central-panel closure, so a naive extraction changes UI flow. Revisit with a dedicated geometry value object.
- Release asset verification is automatic only for assets present when the release event fires. Manual installer/tools attachments require a `workflow_dispatch` re-run of `Verify Release`; a fully automatic guard would require publishing the release only after all assets are attached, which is a release-process decision.
