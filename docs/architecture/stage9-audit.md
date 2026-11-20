# Stage 9: minimised auditing (E9-03)

| Field | Value |
|---|---|
| Package | E9-03 of the [Stage 9 minutes](stage9-record.md) (§3) |
| Invariants | I-29 (extended to `kerno-units`), I-37 |
| Findings it closes | H-6 and H-8 of the [closure of the preview](stage9-early-closure.md) |
| Decision applied | D8: no persisted audit record; only the text that the layer already emits is verified |

## 1. What is verified

`kerno-storage` and `kerno-units` have no logging nor auditing layer. What they emit is three things, and all three are searched:

1. **Errors:** `Display` and `Debug` of each variant that the test manages to produce.
2. **Reports:** `UnitVerifyReport`, `VerifyFinding` and the error that `VerifyFinding::to_error` gives, with each damage caused separately; the retention report.
3. **Observations:** `ObservationStats`, the observer's records and the explanations, at the four levels.

What a correct read returns is the content the caller asked for and is not searched: it carries sentinels on purpose.

## 2. Tests

| File | What it does |
|---|---|
| `crates/kerno-units/tests/redaction_units.rs` (new) | Same detector as `kerno-storage/tests/redaction.rs`: four sentinels (two text, two non-UTF-8 binaries) in type, metadata, payload, actor, source, reason and correlation; forms text, hexadecimal, decimal list and `\x`; fragments of 8 bytes or more. Provokes eight classes of error, six different damages of records and the four observation levels, with `emulated_log` and with `kerno_db`. Fails if an error class, a finding or an observation record was not produced |
| `crates/kerno-storage/tests/redaction.rs` (only the fragment search) | `find_sentinels` also searches each 8-byte window of each sentinel in each form (H-6). The text forms of a window that is not UTF-8 are skipped: they would be replacement characters |
| `crates/kerno-storage/tests/group_commit.rs` (only the comment) | H-8: the comment said "10^4 commits"; they are 4 × 1,500 = 6,000 and 4 × 1,000 = 4,000 in the two rounds |

The detector of each file has its own test: it finds each form and each fragment, it does not find a clean text, and it does not find 7 bytes of a sentinel (one less than the fragment).

## 3. Mutations

Applied to a copy of the tree (without `target` or `.git`), one at a time, in release mode, and the copy was deleted afterwards. Output: the `FAILED` line of each test.

| # | File and line | Change | Result |
|---|---|---|---|
| M1 | `crates/kerno-units/src/db.rs:226` | the message of "actor of N bytes exceeds the maximum" adds `&v[..10]` (the whole binary sentinel) | `emulated_log::units_leak_no_content_in_errors_reports_or_observations` and `kerno_db::…` in `FAILED`; the detector stays green |
| M2 | `crates/kerno-units/src/db.rs:226` | the same, with `&v[2..10]` (a fragment of 8 bytes) | the two tests per engine in `FAILED` ("3 of 252 texts carry a sentinel (I-29)"); the detector stays green |
| M3 | `crates/kerno-storage/src/sst.rs:335` | `Table::open` adds the first 12 bytes of `meta.smallest` to the message (a fragment, not the whole key) | the five configuration tests in `FAILED` ("N of ~18,000 texts carry a sentinel (I-29)"); the detector stays green. With the earlier search (whole key) this mutation would have passed: it is the test of H-6 |
| M4 | `crates/kerno-storage/tests/redaction.rs:118` | `windows(FRAGMENT)` becomes `.take(0)` (the detector does not search fragments) | `detector_finds_every_form_of_every_sentinel` in `FAILED` |
| M5 | `crates/kerno-units/tests/redaction_units.rs:92` | same as M4 | `detector_finds_every_form_and_fragment_of_every_sentinel` in `FAILED` |

## 4. Limits

- The reports of `verify_units` and the observations have no mutation of their own that leaks a sentinel: the text of `VerifyFinding` carries only record keys (identifiers), not content, so there is no production line that the test could see failing without inventing a leak. What the test checks for those paths is that they were exercised (findings and observation records non-zero) and that no text carries a sentinel.
- A fragment of less than 8 bytes is not searched: a shorter window produces matches with legitimate text (numbers, rule names).
- The sentinels are the ones the test plants. Content that does not pass through them is not seen.
- This is not an audit record (D8).
