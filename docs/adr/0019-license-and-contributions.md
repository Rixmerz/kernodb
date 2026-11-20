# ADR-0019 — License and contributions

| Field | Value |
|---|---|
| Status | **Accepted** by the project owner on 2026-11-11 |
| Date | 2026-11-11 |
| Deciders | Project owner |
| Related to | ADR-0010 (public API and crate boundaries); [`Cargo.toml`](../../Cargo.toml), [`LICENSE-MIT`](../../LICENSE-MIT), [`LICENSE-APACHE`](../../LICENSE-APACHE), [`CONTRIBUTING.md`](../../CONTRIBUTING.md) and the "License" section of [`README.md`](../../README.md) |

## Context

The engine is an embedded library. Today the license is declared in two places that agree: `license = "MIT OR Apache-2.0"` in `[workspace.package]` of `Cargo.toml`, which `kerno-core` and `kerno-storage` inherit with `license.workspace = true`, and the texts `LICENSE-MIT` and `LICENSE-APACHE`. The "License" section of the README adds that a contribution submitted for inclusion is licensed under those two licenses, "without any additional terms or conditions". No contribution agreement exists: neither the README nor `CONTRIBUTING.md` mentions one.

The decision must be made now because the terms of each external contribution are fixed with its first submission. Without a prior agreement, the owner will not be able to relicense other people's code or offer commercial terms without asking each author for permission. It is also necessary to fix where revenue may come from, if at all, and when charging starts to be considered.

This decision does not change the format or the API: the v1 format has been frozen since 2026-10-31 ([ADR-0013](0013-format-v1-and-engine-protocol.md)) and the crate boundaries are accepted ([ADR-0010](0010-public-api-and-crate-boundaries.md)).

## Options considered

1. **Permissive, "MIT OR Apache-2.0" (chosen).** Anyone can use, modify and integrate the core into closed products without publishing their code. *Adoption:* the highest, and the least legal friction. *Who pays:* nobody for the engine; revenue, if any, comes from what is outside the core. *Cost:* whoever offers the engine as a service does not have to pay for it.
2. **AGPL-3.0 with a commercial license.** The core obliges whoever offers it as a network service to publish their changes. *Adoption:* lower in organizations with a policy against AGPL, which are also the ones that buy. *Who pays:* whoever cannot comply with AGPL buys the commercial license. It requires a CLA, because the commercial license requires being able to license all authors' code.
3. **Source-visible license with deferred conversion.** The code is published, but commercial use is restricted for a period and then moves to an open license. *Adoption:* lower, because it is not an approved open-source license and many organizations ban it. *Who pays:* whoever uses the engine during the restricted period. It requires a CLA.
4. **Open core and paid enterprise features.** The core is permissive; enterprise features are distributed under a proprietary license. *Adoption:* high in the core, but the boundary between open and paid creates friction with contributors. *Who pays:* whoever needs the enterprise features. It requires separating the code and a CLA to maintain both parts.

## Decision criteria

The criteria that the owner set when deciding:

- adoption without legal friction for whoever uses the core;
- that the owner keeps the option to relicense and to offer commercial terms;
- that revenue, if any, comes from something other than the engine;
- that the engine is not charged for before a 1.0 with a stable format, migrations and real users in production.

## Evidence

- `Cargo.toml` declares `license = "MIT OR Apache-2.0"` in `[workspace.package]`; `crates/kerno-core/Cargo.toml` and `crates/kerno-storage/Cargo.toml` inherit the license with `license.workspace = true`. Command: `grep -n license Cargo.toml crates/kerno-core/Cargo.toml crates/kerno-storage/Cargo.toml`.
- The "License" section of the README licenses the contribution under the two licenses, without additional terms. It does not mention any CLA; neither does `CONTRIBUTING.md`.
- The v1 format is frozen and ADR-0013 was accepted on 2026-10-31 ([`v1.md`](../file-format/v1.md)). ADR-0010 was accepted on 2026-10-31.
- Migrations with fixtures from earlier versions are outside the Stage 9 advance and remain pending ([advance minutes, §2.3](../architecture/stage9-early-record.md)).

## Decision

The core is distributed under "MIT OR Apache-2.0", as `Cargo.toml` and the texts `LICENSE-MIT` and `LICENSE-APACHE` declare. Before the first external contribution is accepted, a contributor license agreement (CLA) is needed that keeps for the owner the option to relicense or to offer commercial terms. If there is revenue, it will come from what is outside the core: synchronization as a service, enterprise features and support. The engine is not charged for, and charging for it is not considered before a 1.0 with a stable format, migrations and real users in production.

## Consequences

- **Easier:** anyone can integrate the core without asking permission or paying; the owner can offer commercial terms to whoever asks for them, provided the CLA covers the code.
- **Harder:** whoever offers Kerno as a service does not pay the owner. Revenue depends on services and features that do not yet exist.
- **Committed:** every external contribution needs the CLA from the first one; without it, the owner cannot relicense that code. Until the CLA exists, the only rule on contributions is the "License" section of the README, which does not mention it.
- **Review:** this ADR, before any charging and before a 1.0.

## Follow-up

- **CLA text:** it is not written. It is a follow-up of this ADR. It must decide whether the contributor assigns their rights or grants a broad license, and it must exist before the first external contribution is accepted.
- **README and `CONTRIBUTING.md`:** when the CLA exists, the "License" section of the README and `CONTRIBUTING.md` must refer to it. Those files are outside `docs/` and are not modified in this task.
- **Charging:** reopen this ADR when the conditions of a 1.0 with a stable format, migrations and real users in production are met.
- **Draft (2026-11-13):** a proposal exists, [cla-draft.md](../legal/cla-draft.md), in draft and not in force. It awaits review by a lawyer and approval by the owner. The "CLA text" point above still holds: the text is not approved.
