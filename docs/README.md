# Kerno documentation

| Folder | Contents |
|---|---|
| [`architecture/`](architecture/) | [SAD v0.1.0](architecture/SAD.md) (baseline), [invariants and risks](architecture/invariants.md), [failure matrix](architecture/failure-matrix.md), [Revision model design](architecture/revision-model-design.md), [Phase 1 design](architecture/phase1-design.md), [adaptive execution](architecture/adaptive-execution.md) |
| [`spec/`](spec/README.md) | [Architecture specification v0.2.0](spec/architecture-specification-v0.2.0.md) and its mapping to phases and ADRs |
| [`adr/`](adr/README.md) | Architecture Decision Records 0001–0021 and template |
| [`file-format/`](file-format/README.md) | Principles of the physical format (not frozen) |
| [`security/`](security/threat-model.md) | Initial threat model |
| [`testing/`](testing/) | [Test plan](testing/test-plan.md) and [M0 report](testing/m0-report.md) |
| [`roadmap/`](roadmap/README.md) | Status by phase and M0 checklist |
| [`guides/`](guides/) | [Kerno as a revisioned history next to another database](guides/history-alongside.md), [one unit per entity](guides/one-unit-per-entity.md) |
| [`bindings-coverage.md`](bindings-coverage.md) | Which calls of the `kerno` facade the Python and Node bindings expose, and under which name |
| [`FEATURES.md`](FEATURES.md) | Feature inventory: the status of every capability, audited against the code and its tests |
| [`COMPLETION-PLAN.md`](COMPLETION-PLAN.md) | Completion plan: every FEATURES.md row that is not implemented, in ordered milestones toward 1.0 |
