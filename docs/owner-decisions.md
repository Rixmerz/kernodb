# Owner decisions, 2026-11-19

This page records three decisions the project owner took on 2026-11-19, and
one list of things the owner did not authorise. It changes no code. Rows in
[COMPLETION-PLAN](COMPLETION-PLAN.md) that depend on these decisions link here.

## Test machine

The final runs (OPS-13 long fuzzing, OPS-06 U2 on disk, OPS-10 recovery time
and SQLite comparison, U6 medians) run on the owner's own computer.

- No run is declared complete and no result is certified until the machine is
  shown to hold a load average below 1 for the whole measurement.
- Before the full cycle:
  1. Record CPU, RAM, storage, OS and versions.
  2. Run a pilot for load and duration.
  3. Record configuration, temperature and load.
  4. Run the targets and benchmarks with the set durations: `wal`, `manifest`
     and `open` for 8 h each; the other 12 targets for 30 min each. That is
     about 30 h of fuzzing.
- If the conditions do not hold, the results are recorded as **inconclusive**.
- Fuzz durations and acceptance thresholds are **not reduced**.

The measured machine and the risks on it are in
[testing/test-machine.md](testing/test-machine.md).

## External users (INT-13, INT-07)

- LiveSpec and the owner's own projects **do not count** as independent
  external users.
- Tests run by agents do not count either.
- Status: **BLOCKED, external validation pending.** The owner will find
  external evaluators; the goal is three. No names are invented here, and the
  requirement is **not** declared met.
- Every report received through [feedback.md](feedback.md) gets a disposition:
  **fixed**, **declared limit**, or **refused with a reason**.

## CLA (ADR-0019)

- **Direction (b) is confirmed**: a CLA with a broad, non-exclusive licence,
  with the right to sublicense and relicense, including commercially, without
  transfer of authorship. The public licence stays MIT OR Apache-2.0.
- It is **not** replaced by a DCO. A DCO alone does not grant the relicensing
  right that ADR-0019 requires.
- The text stays a **DRAFT, NOT IN FORCE**: [legal/cla-draft.md](legal/cla-draft.md).
- Before approval of the legal text:
  - define the owner's legal identity (natural person or legal entity);
  - review the patent section (beneficiaries, termination on litigation);
  - define governing law and jurisdiction (the project is developed from Chile);
  - check that the Spanish and English versions have the same legal effect;
  - obtain a professional legal review.
- No external contributions are accepted until an approved, dated and
  published version exists.
- The direction is approved; the legal text is not.

## Not authorised

- Declaring 1.0.
- Publishing to npm or to any other registry.
