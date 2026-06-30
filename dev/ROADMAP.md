# jit-lang - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: ../../_strategy/LANG_COLLECTION.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt.

## v0.2.0 - Core (THE HARD PART, NOT DEFERRED) (DONE)
Lowers IR to machine code in executable memory via Cranelift and runs it immediately.
Dependencies (wires ir, pager, Cranelift) are wired here, when first used.
Exit criteria:
- [x] Every public item has rustdoc + a runnable example.
- [x] Core invariants property-tested (API authored at this stage in docs/API.md).

Decisions recorded here per the anti-deferral rule:
- Cranelift 0.133 is the code generator (cranelift-codegen + cranelift-frontend +
  cranelift-native); pager-lang hosts the emitted bytes in a read-execute region. No
  cranelift-jit, so memory management stays with pager-lang as planned.
- MSRV raised to 1.94 — the floor cranelift-codegen 0.133 requires. This is the only
  crate in the family above the 1.85 baseline; the dependency imposes it.
- Scope held to leaf functions over int/float/bool with a unit return. A unit-typed
  parameter is rejected (no machine representation), not silently lowered.

## v1.0.0 - API freeze
Public surface stable and frozen until 2.0.
- [ ] docs/API.md marked stable; SemVer promise recorded.
- [ ] Full test + benchmark suite green on all three platforms.
