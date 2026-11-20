# Stage 5 · E5-04 — Sentinels

Contract of E5-04 ([Stage 5 minutes](stage5-record.md) §3; [ADR-0015](../adr/0015-privacy-of-observability.md), I-37, I-29, IU-10). The test is in `crates/kerno-units/tests/sentinels.rs`.

## 1. What is planted and where it is searched

A sentinel is a string that no Kerno layer would write by itself. It is planted at each point through which user content enters:

| Point | Sentinel |
|---|---|
| Unit type | `SNT-TYPE-…` |
| Payload and metadata | `SNT-PAYLOAD-…`, `SNT-METADATA-…` |
| Relation type | `SNT-RELTYPE-…` |
| Provenance | source (`Imported`) and origin (`Algorithmic`, `Inferred`) |
| Filter values | the type, and a 10-digit schema version |
| Oversized type (write error) | `SNT-OVERSIZE-…` |

Each one is searched for in three forms, the ones it would have in a `Display` or a `Debug`: the text, the decimal list of bytes of a `Vec<u8>` and the hex.

The search covers everything the layer says about a query: the records (`{:#?}`) and the observer's counters, `explain_filter` and `explain_relations` (the plan, its candidates, the footprint in `Debug` and `Display`), the paths and fallbacks of the answer, and the errors of queries, of the index rebuild and of a rejected write (`Display` and `Debug`). It is repeated in `off`, `aggregate`, `fingerprint` and `diagnostic`, without and with a damaged unit record (the read fails with corruption).

The minutes ask for no sentinel outside `diagnostic`. The text of the shape that `diagnostic` adds carries typed markers (`units[unit_type=?text]`), so the test requires more: none at any level. Whoever makes `diagnostic` carry a literal on purpose relaxes that single assertion and does not touch the lower levels.

## 2. Tests

| Test | What it shows |
|---|---|
| `no_sentinel_in_any_observability_explanation_or_error_output` | The search above. Also, each reported footprint (plans and records, at each level) equals that of the same shape with other literals: a literal that enters the hash input never appears as text and the search alone would not see it. With a control: the answer to the caller does contain the sentinels, and the searched output is not empty (there are records from `fingerprint`, the shape in `diagnostic`, the rejected oversized write and at least one query error) |
| `levels_below_diagnostic_carry_no_shape_text` | Structural half of I-37: `fingerprint_text` only exists in `diagnostic` and is the shape with markers |

Each one runs on `Emulated<LogEngine>` and on `KernoDb`.

## 3. Test count

Sums of the `test result` lines of each run, before (commit of E5-03) and after this package; no test fails in either of the two:

| Suite | Before | After |
|---|---|---|
| `cargo test --workspace --exclude kerno-storage --all-features --release` | **611** pass, 8 ignored | **615** pass, 8 ignored |
| `cargo test -p kerno-storage --all-features --release` | 649 pass, 4 ignored | 649 pass, 4 ignored (unchanged: this package does not touch `kerno-storage`) |
| Full suite (sum of the two) | **1260** | **1264** |

+4: two tests for two engines, `Emulated<LogEngine>` and `KernoDb`. The binary `tests/sentinels.rs` reports `4 passed`. The starting 611 matches the closure of [E5-03](stage5-observation.md) §6.

## 4. The test can fail

Each new test was seen to fail. Inverting the search assertion (`!text.contains` → `text.contains`, only in `tests/sentinels.rs`): the 2 search tests fail. Mutations "copy the literal into the footprint or into the explanation" from the minutes, applied to the production code and restored with `git checkout` (`cargo test -p kerno-units --all-features --release --test sentinels`):

| Mutation | Result |
|---|---|
| Copy the filter's type into the units shape (`NormalizedQuery::of_filter`) | all 4 fail: the footprint differs from the shape without literals already in `Off` (`fingerprint units/… at level Off depends on a literal`), and the `diagnostic` shape is not the expected one. With the footprint check disabled, the text search also fails: `unit type sentinel "SNT-TYPE-7f3a91c2" leaked at level Diagnostic` |
| Copy the relation type into the relations shape (`NormalizedQuery::of_relations`) | all 4 fail: `fingerprint relations/… at level Off depends on a literal` and the `diagnostic` shape |
| Copy the unit or relation type only into the hash input (new field in `NormalizedQuery`, `hasher.update`), without touching the text | the 2 search tests fail through the footprint check, in `Off`. **Finding:** with the text search alone, the 4 stay green; that is why the check was added |
| Copy the filter's schema version into the candidate cost of the reference (`unit_candidates`, the explanation) | the 2 search tests fail: `schema version sentinel "4071337791" leaked at level Off` (`Cost(4071337791)` in the plan) |

## 5. Threat model review

[T-11](../security/threat-model.md) covers observations and plan explanations: adversaries, response and residue. What the test does **not** guarantee: that a new field does not leak through a channel the test does not read (which is why ADR-0015 requires extending it with each field), the process memory and the key (out of scope), and the counters and durations as a side channel.
