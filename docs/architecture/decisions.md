# Architecture decision records

## ADR-0001 — Refactor in an isolated worktree

- Status: accepted
- Context: a systematic, multi-round refactor must not disturb the active main branch or parallel work.
- Decision: perform all work in `C:/tmp/iv-arch` on `arch/systematic-refactor`; publish each completed batch as an independent commit.
- Consequences: main remains usable throughout; integration requires explicit merge/PR per batch.

## ADR-0002 — Preserve external contracts by default

- Status: accepted
- Context: settings format, file associations, CLI flags, environment variables, release assets, and tests are existing external contracts.
- Decision: no contract changes in a refactoring batch. Any necessary breaking change is recorded as "awaiting confirmation" and work continues on independent items.
- Consequences: refactors stay behavior-preserving and independently revertable.

## ADR-0003 — Deletion test before new abstraction

- Status: accepted
- Context: entropy reduction must not replace one layer of indirection with another.
- Decision: introduce a helper/module only if deleting it would push complexity back into multiple callers; remove superseded implementations in the same batch.
- Consequences: favors standardization and deletion over parallel implementations.
